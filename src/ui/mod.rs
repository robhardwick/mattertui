//! The dashboard: terminal setup, event loop and top-level drawing.

pub mod app;
pub mod icons;
pub mod layout;
pub mod motion;
pub mod series;
pub mod theme;
pub mod widgets;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use self::app::{Action, App, View};
use self::theme::Theme;
use self::widgets::{chrome, detail, modal, room, tile};
use crate::config::Config;
use crate::logbuf::LogBuffer;
use crate::matter::readonly::ReadOnlyNode;
use crate::matter::worker::{self, WorkerSpec};
use crate::matter::{self, Controller, Trust, pairing};
use crate::paths::Paths;
use crate::state::{AppEvent, DeviceUpdate, Status};

pub async fn run(paths: &Paths, config: Config, logs: Arc<LogBuffer>) -> Result<()> {
    let _lock = paths.lock()?;
    // The log panel isn't on screen yet, so say why we might be waiting.
    if !matter::clock_is_sane() {
        eprintln!("waiting for the system clock to be set (NTP)…");
    }
    let ctl = matter::open(paths, Trust::IfAvailable).await?;
    let (tx, rx) = mpsc::channel::<AppEvent>(1024);
    let mut app = App::new(config, logs, Theme::detect());
    app.can_pair = ctl.can_commission;
    let mut rt = Runtime {
        ctl,
        tx,
        workers: HashMap::new(),
    };

    let nodes = rt.ctl.inner.nodes().await?;
    for i in 0..app.config.devices.len() {
        let id = app.config.devices[i].id.get_ref().clone();
        match nodes.iter().find(|n| n.label.as_deref() == Some(id.as_str())) {
            Some(n) => {
                app.devices[i].apply(DeviceUpdate::Status(Status::Connecting));
                rt.start_worker(&app, i, n.node_id);
            }
            None => tracing::warn!(device = id, "not paired; select it and press p to pair"),
        }
    }
    for n in &nodes {
        let label = n.label.as_deref().unwrap_or("");
        if app.config.device(label).is_none() {
            tracing::warn!(
                node_id = format_args!("0x{:016X}", n.node_id),
                label,
                "paired device has no [[device]] entry in the config"
            );
        }
    }

    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, &mut app, &mut rt, rx).await;
    ratatui::restore();
    result
}

/// What the event loop needs to act on devices: the controller, the event
/// channel, and a handle per running device worker.
struct Runtime {
    ctl: Controller,
    /// Also keeps the event channel open while no workers are running.
    tx: mpsc::Sender<AppEvent>,
    workers: HashMap<usize, AbortHandle>,
}

impl Runtime {
    fn start_worker(&mut self, app: &App, device: usize, node_id: u64) {
        let dev = &app.config.devices[device];
        let handle = worker::spawn(
            WorkerSpec {
                device_id: dev.id.get_ref().clone(),
                kind: dev.kind,
                endpoint: dev.endpoint,
            },
            ReadOnlyNode::new(self.ctl.inner.node(node_id)),
            self.tx.clone(),
        );
        if let Some(old) = self.workers.insert(device, handle) {
            old.abort();
        }
    }

    /// Start pairing/unpairing in the background; the outcome comes back
    /// as an `AppEvent`.
    fn start(&mut self, app: &App, action: Action) {
        let ctl = self.ctl.clone();
        let tx = self.tx.clone();
        match action {
            Action::Pair { device, code } => {
                let id = app.config.devices[device].id.get_ref().clone();
                tokio::spawn(async move {
                    let result = pairing::pair(&ctl, &id, &code)
                        .await
                        .map(|info| info.node_id)
                        .map_err(|e| {
                            tracing::warn!(device = id, "pairing failed: {e:#}");
                            friendly_error(&e)
                        });
                    let _ = tx.send(AppEvent::Paired { id, result }).await;
                });
            }
            Action::Unpair { device } => {
                // Stop the worker first so its subscription goes away.
                if let Some(h) = self.workers.remove(&device) {
                    h.abort();
                }
                let id = app.config.devices[device].id.get_ref().clone();
                tokio::spawn(async move {
                    let result = pairing::unpair(&ctl, &id)
                        .await
                        .map(|removed| {
                            if removed {
                                "Removed from the device. You can pair it again with a new setup code.".to_string()
                            } else {
                                format!(
                                    "Forgotten locally, but the device couldn't be reached. Remove \"{}\" \
                                     under the device's Connected Services in the Home app.",
                                    pairing::FABRIC_LABEL
                                )
                            }
                        })
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(AppEvent::Unpaired { id, result }).await;
                });
            }
        }
    }

    /// React to outcomes that need the controller (before the app sees them).
    fn on_event(&mut self, app: &App, event: &AppEvent) {
        if let AppEvent::Paired { id, result: Ok(node_id) } = event
            && let Some(device) = app.config.devices.iter().position(|d| d.id.get_ref() == id)
        {
            self.start_worker(app, device, *node_id);
        }
    }
}

/// Turn a pairing error into a sentence for the dialog.
fn friendly_error(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    if text.contains("not found via mDNS") {
        "Couldn't find the device in pairing mode. Check pairing mode is still on in the Home app \
         (it times out after a few minutes), then try again."
            .into()
    } else if text.contains("PASE") || text.contains("passcode") {
        format!("The device rejected the code. Get a fresh code from the Home app and try again. ({text})")
    } else {
        text
    }
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rt: &mut Runtime,
    mut rx: mpsc::Receiver<AppEvent>,
) -> Result<()> {
    let logs = app.logs.clone();
    let mut events = crossterm::event::EventStream::new();
    loop {
        terminal.draw(|f| draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        if let Some(action) = app.action.take() {
            rt.start(app, action);
        }
        // Smooth while animating, gentle for ambient motion, and once a
        // second otherwise (clock, "ago" labels, staleness).
        let wait = if app.animating() {
            Duration::from_millis(33)
        } else if app.ambient() || app.ticker_scrolling {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(1)
        };
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(e)) => app.on_terminal_event(e),
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(event) = rx.recv() => {
                rt.on_event(app, &event);
                app.on_app_event(event);
                while let Ok(event) = rx.try_recv() {
                    rt.on_event(app, &event);
                    app.on_app_event(event);
                }
            }
            () = logs.notify.notified() => {}
            () = tokio::time::sleep(wait) => {}
        }
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let log_height = if app.log_open {
        app.config.log.height.saturating_add(1).min(area.height / 2)
    } else {
        0
    };
    let [top, body, ticker, log] =
        Rect::new(area.x, area.y, area.width, area.height).layout(&ratatui::layout::Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(log_height),
        ]));
    let buf = frame.buffer_mut();
    chrome::top_bar(app, top, buf);
    app.ticker_scrolling = chrome::ticker(app, ticker, buf);
    match app.view {
        View::Dashboard => dashboard(app, body, buf),
        View::Detail => detail::render(app, body, buf),
    }
    if app.log_open {
        chrome::log_panel(app, log, buf);
    }
    if let Some(m) = &app.modal {
        modal::render(app, m, area, buf);
    }
    if app.show_help {
        chrome::help(app, area, buf);
    }
}

/// Render the dashboard off-screen at full height, then copy the visible
/// window into the frame. This makes scrolling trivial and keeps widgets
/// from ever drawing outside their buffer.
fn dashboard(app: &mut App, body: Rect, buf: &mut Buffer) {
    if app.config.rooms.is_empty() || app.config.devices.is_empty() {
        Line::styled("No rooms or devices in the config yet. See config.example.toml.", app.theme.muted())
            .alignment(Alignment::Center)
            .render(Rect::new(body.x, body.y + body.height / 2, body.width, 1), buf);
        return;
    }
    app.relayout(body);
    let full = Rect::new(0, 0, body.width, app.layout.height.max(body.height));
    let mut off = Buffer::empty(full);
    for r in &app.layout.rooms {
        room::render(app, r.room, r.rect, &mut off);
    }
    for t in &app.layout.tiles {
        let device = app.device_at(t.room, t.tile);
        tile::render(app, device, t.variant, room::accent(app, t.room), t.rect, &mut off);
    }
    // Startup: rooms sweep in left to right, one after another, behind a
    // bright scan line in the room's accent colour.
    for r in &app.layout.rooms {
        let p = app.reveal(r.room);
        if p >= 1.0 {
            continue;
        }
        let edge = r.rect.x + (p * f64::from(r.rect.width)).round() as u16;
        let accent = room::accent(app, r.room);
        for y in r.rect.top()..r.rect.bottom() {
            for x in edge..r.rect.right() {
                off[(x, y)].reset();
            }
            if edge < r.rect.right() {
                off[(edge, y)].set_char('▏').set_fg(accent);
            }
        }
    }
    for y in 0..body.height {
        for x in 0..body.width {
            buf[(body.x + x, body.y + y)] = off[(x, y + app.scroll)].clone();
        }
    }
    if app.layout.height > body.height {
        let more = if app.scroll + body.height < app.layout.height { "↓ more " } else { "↑ " };
        Line::styled(more, ratatui::style::Style::new().fg(app.theme.dim()))
            .alignment(Alignment::Right)
            .render(Rect::new(body.x, body.bottom() - 1, body.width, 1), buf);
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;
    use crate::state::{DeviceInfo, Metric, Transport, unix_minute};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const CONFIG: &str = r#"
[[room]]
id = "living-room"
name = "Living room"
icon = "sofa"

[[room]]
id = "bedroom"
name = "Bedroom"
icon = "bed"

[[room]]
id = "kitchen"
name = "Kitchen"
icon = "kitchen"

[[device]]
id = "lamp"
name = "Floor lamp"
room = "living-room"
kind = "plug"

[[device]]
id = "heating"
name = "Heating"
room = "living-room"
kind = "thermostat"
featured = true

[[device]]
id = "lounge-temp"
name = "Sofa sensor"
room = "living-room"
kind = "temperature_sensor"

[[device]]
id = "bedside"
name = "Bedside"
room = "bedroom"
kind = "temperature_sensor"

[[device]]
id = "fairy-lights"
name = "Fairy lights"
room = "bedroom"
kind = "plug"
icon = "star"

[[device]]
id = "kettle"
name = "Kettle"
room = "kitchen"
kind = "plug"
icon = "plug"
"#;

    /// A plausible house: live readings plus an hour of history.
    pub fn fake_app() -> App {
        let config = Config::parse(CONFIG).unwrap();
        let mut app = App::new(config, LogBuffer::new(), Theme { truecolor: true, ascii: false });
        let now = unix_minute();
        let live = Status::Live { max_interval_s: 60 };
        let info = |transport| DeviceInfo {
            node_id: 2,
            vendor: Some("Acme".into()),
            product: Some("Gadget".into()),
            firmware: Some("1.0".into()),
            transport,
            endpoint: 1,
        };
        let set = |app: &mut App, i: usize, m: Metric, f: &dyn Fn(i64) -> f64| {
            let h = app.devices[i].history.entry(m).or_insert_with(|| crate::state::History::new(1440));
            for k in (0..60).rev() {
                h.record(now - k, f(k));
            }
            app.devices[i].current.insert(m, Some(f(0)));
        };
        for i in 0..6 {
            app.devices[i].status = live.clone();
        }
        app.devices[0].info = Some(info(Transport::WiFi));
        set(&mut app, 0, Metric::On, &|k| if k < 25 { 1.0 } else { 0.0 });
        app.devices[1].info = Some(info(Transport::Thread));
        set(&mut app, 1, Metric::Temperature, &|k| 19.0 + (60 - k) as f64 * 0.025);
        set(&mut app, 1, Metric::HeatSetpoint, &|_| 21.0);
        set(&mut app, 1, Metric::Running, &|k| if k < 40 { 1.0 } else { 0.0 });
        set(&mut app, 1, Metric::Mode, &|_| 4.0);
        set(&mut app, 2, Metric::Temperature, &|k| 20.5 + ((k as f64) / 9.0).sin() * 0.6);
        set(&mut app, 2, Metric::Humidity, &|_| 48.0);
        app.devices[3].info = Some(info(Transport::Thread));
        set(&mut app, 3, Metric::Temperature, &|k| 17.2 + (k as f64) * 0.01);
        set(&mut app, 3, Metric::Humidity, &|_| 55.0);
        set(&mut app, 3, Metric::Battery, &|_| 15.0);
        set(&mut app, 4, Metric::On, &|_| 1.0);
        app.devices[5].status = Status::Offline { error: "no route".into() };
        app.selected = 1;
        // Snapshots must not depend on timing.
        app.motion = false;
        app
    }

    pub fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Strip the volatile clock so snapshots are stable.
    pub fn stable(s: String) -> String {
        let mut lines: Vec<String> = s.lines().map(str::to_string).collect();
        // The top bar ends with the clock, "HH:MM".
        if let Some(top) = lines.first_mut()
            && let Some((cut, _)) = top.char_indices().rev().nth(4)
        {
            top.replace_range(cut.., "HH:MM");
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::app::View;
    use super::tests_support::*;

    #[test]
    fn dashboard_wide() {
        let mut app = fake_app();
        insta::assert_snapshot!(stable(render(&mut app, 130, 34)));
    }

    #[test]
    fn dashboard_small_with_log() {
        let mut app = fake_app();
        app.log_open = true;
        insta::assert_snapshot!(stable(render(&mut app, 80, 24)));
    }

    #[test]
    fn detail_view() {
        let mut app = fake_app();
        app.view = View::Detail;
        insta::assert_snapshot!(stable(render(&mut app, 100, 30)));
    }
}

#[cfg(test)]
mod reconnect_tests {
    use super::app::Health;
    use super::tests_support::*;
    use crate::state::Status;
    use std::time::{Duration, Instant};

    #[test]
    fn brief_reconnect_is_subtle_then_escalates() {
        let mut app = fake_app();
        app.devices[2].status = Status::Reconnecting { cause: "timeout".into() };
        app.devices[2].status_since = Instant::now();
        assert_eq!(app.health(2), Health::Reconnecting);
        assert!(app.health(2).is_ok());
        let screen = render(&mut app, 130, 34);
        assert!(!screen.contains("reconnecting"), "no loud text on the dashboard");
        assert!(screen.contains("5/6 live"), "still counted as live");

        app.devices[2].status_since = Instant::now() - Duration::from_secs(600);
        assert_eq!(app.health(2), Health::Stale);
    }
}

#[cfg(test)]
mod pairing_tests {
    use super::app::{Action, ModalKind, Phase};
    use super::tests_support::*;
    use crate::state::{AppEvent, Status};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    fn key(app: &mut super::App, code: KeyCode) {
        app.on_terminal_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn type_str(app: &mut super::App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c));
        }
    }

    #[test]
    fn pair_flow_from_keypress_to_done() {
        let mut app = fake_app();
        app.can_pair = true;
        app.devices[3].status = Status::Unpaired;
        app.selected = 3;

        key(&mut app, KeyCode::Char('p'));
        let m = app.modal.as_ref().unwrap();
        assert_eq!((m.kind, &m.phase), (ModalKind::Pair, &Phase::Input { error: None }));

        // A bad code stays in the dialog with an error; no action starts.
        type_str(&mut app, "3497-011-2333");
        key(&mut app, KeyCode::Enter);
        assert!(app.action.is_none());
        assert!(matches!(app.modal.as_ref().unwrap().phase, Phase::Input { error: Some(_) }));

        key(&mut app, KeyCode::Backspace);
        type_str(&mut app, "2");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.action, Some(Action::Pair { device: 3, code: "34970112332".into() }));
        assert!(app.animating());

        app.on_app_event(AppEvent::Paired { id: "bedside".into(), result: Ok(7) });
        assert!(matches!(app.modal.as_ref().unwrap().phase, Phase::Done { ok: true, .. }));
        assert_eq!(app.devices[3].status, Status::Connecting);
        key(&mut app, KeyCode::Enter);
        assert!(app.modal.is_none());
    }

    #[test]
    fn failed_pairing_returns_to_input_for_retry() {
        let mut app = fake_app();
        app.can_pair = true;
        app.devices[3].status = Status::Unpaired;
        app.selected = 3;
        key(&mut app, KeyCode::Char('p'));
        type_str(&mut app, "34970112332");
        key(&mut app, KeyCode::Enter);
        app.on_app_event(AppEvent::Paired { id: "bedside".into(), result: Err("not found".into()) });
        let m = app.modal.as_ref().unwrap();
        assert_eq!(m.input, "34970112332", "code kept for retry");
        assert_eq!(m.phase, Phase::Input { error: Some("not found".into()) });
    }

    #[test]
    fn pairing_needs_roots_and_an_unpaired_device() {
        let mut app = fake_app();
        app.selected = 0; // paired
        key(&mut app, KeyCode::Char('p'));
        assert!(matches!(app.modal.as_ref().unwrap().phase, Phase::Done { ok: false, .. }));
        key(&mut app, KeyCode::Esc);

        app.devices[0].status = Status::Unpaired;
        key(&mut app, KeyCode::Char('p')); // can_pair is false
        let Phase::Done { message, .. } = &app.modal.as_ref().unwrap().phase else { panic!() };
        assert!(message.contains("fetch-roots"));
    }

    #[test]
    fn unpair_confirms_then_resets_device() {
        let mut app = fake_app();
        app.selected = 0;
        key(&mut app, KeyCode::Char('U'));
        key(&mut app, KeyCode::Char('y'));
        assert_eq!(app.action, Some(Action::Unpair { device: 0 }));
        app.on_app_event(AppEvent::Unpaired { id: "lamp".into(), result: Ok("Removed".into()) });
        assert_eq!(app.devices[0].status, Status::Unpaired);
        assert!(app.devices[0].current.is_empty());
    }

    #[test]
    fn modal_snapshot() {
        let mut app = fake_app();
        app.can_pair = true;
        app.devices[3].status = Status::Unpaired;
        app.selected = 3;
        key(&mut app, KeyCode::Char('p'));
        type_str(&mut app, "3497-011-2");
        insta::assert_snapshot!(stable(render(&mut app, 100, 30)));
    }
}

#[cfg(test)]
mod motion_tests {
    use super::tests_support::*;
    use crate::state::{AppEvent, DeviceUpdate, Metric, Status};

    fn reading(app: &mut super::App, id: &str, metric: Metric, v: f64) {
        app.on_app_event(AppEvent::Device {
            id: id.into(),
            update: DeviceUpdate::Reading { metric, value: Some(v) },
        });
    }

    #[test]
    fn readings_tween_and_flash_when_motion_is_on() {
        let mut app = fake_app();
        app.motion = true;
        let before = app.shown(3, Metric::Temperature).unwrap();
        reading(&mut app, "bedside", Metric::Temperature, before + 3.0);
        let mid = app.shown(3, Metric::Temperature).unwrap();
        assert!(mid >= before && mid < before + 3.0, "starts near the old value: {mid}");
        assert!(app.flash(3) > 0.9);
        assert!(app.animating());

        app.motion = false;
        assert_eq!(app.shown(3, Metric::Temperature), Some(before + 3.0));
        assert_eq!(app.flash(3), 0.0);
    }

    #[test]
    fn ticker_records_notable_changes_only() {
        let mut app = fake_app();
        reading(&mut app, "lamp", Metric::On, 0.0); // was on
        reading(&mut app, "bedside", Metric::Temperature, 17.3); // small wobble: first sample
        reading(&mut app, "bedside", Metric::Temperature, 17.4); // still small
        reading(&mut app, "bedside", Metric::Temperature, 18.0); // ≥ 0.5 from 17.3
        app.on_app_event(AppEvent::Device { id: "heating".into(), update: DeviceUpdate::Status(Status::Offline { error: "x".into() }) });
        app.on_app_event(AppEvent::Device { id: "heating".into(), update: DeviceUpdate::Status(Status::Connecting) });
        app.on_app_event(AppEvent::Device { id: "heating".into(), update: DeviceUpdate::Status(Status::Live { max_interval_s: 60 }) });
        let texts: Vec<_> = app.ticker.events().map(|e| e.text.clone()).collect();
        assert_eq!(
            texts,
            ["Heating back online", "Heating offline", "Bedside 18.0° ↑", "Floor lamp off"]
        );
    }

    #[test]
    fn startup_sweep_reveals_rooms_in_turn() {
        let mut app = fake_app();
        app.motion = true;
        app.epoch = std::time::Instant::now();
        assert!(app.reveal(0) < 1.0);
        assert!(app.reveal(2) <= app.reveal(0));
        app.epoch -= std::time::Duration::from_secs(5);
        assert_eq!(app.reveal(2), 1.0);
    }

    #[test]
    fn long_ticker_scrolls_only_with_motion() {
        let mut app = fake_app();
        for _ in 0..10 {
            reading(&mut app, "lamp", Metric::On, 1.0);
            reading(&mut app, "lamp", Metric::On, 0.0);
        }
        render(&mut app, 80, 24);
        assert!(!app.ticker_scrolling);
        app.motion = true;
        app.epoch -= std::time::Duration::from_secs(10); // past the startup sweep
        render(&mut app, 80, 24);
        assert!(app.ticker_scrolling);
    }
}

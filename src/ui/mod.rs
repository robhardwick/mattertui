//! The dashboard: terminal setup, event loop and top-level drawing.

pub mod app;
pub mod icons;
pub mod layout;
pub mod series;
pub mod theme;
pub mod widgets;

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

use self::app::{App, View};
use self::theme::Theme;
use self::widgets::{chrome, detail, room, tile};
use crate::config::Config;
use crate::logbuf::LogBuffer;
use crate::matter::readonly::ReadOnlyNode;
use crate::matter::worker::{self, WorkerSpec};
use crate::matter::{self, Trust};
use crate::paths::Paths;
use crate::state::{AppEvent, DeviceUpdate, Status};

pub async fn run(paths: &Paths, config: Config, logs: Arc<LogBuffer>) -> Result<()> {
    let _lock = paths.lock()?;
    let ctl = matter::open(paths, Trust::NotNeeded).await?;
    let (tx, rx) = mpsc::channel::<AppEvent>(1024);
    let mut app = App::new(config, logs, Theme::detect());

    let nodes = ctl.inner.nodes().await?;
    for (i, dev) in app.config.devices.iter().enumerate() {
        let id = dev.id.get_ref();
        match nodes.iter().find(|n| n.label.as_deref() == Some(id.as_str())) {
            Some(n) => {
                app.devices[i].apply(DeviceUpdate::Status(Status::Connecting));
                worker::spawn(
                    WorkerSpec {
                        device_id: id.clone(),
                        kind: dev.kind,
                        endpoint: dev.endpoint,
                    },
                    ReadOnlyNode::new(ctl.inner.node(n.node_id)),
                    tx.clone(),
                );
            }
            None => tracing::warn!(device = id, "not paired; run `mattertui pair {id} <code>`"),
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
    let result = event_loop(&mut terminal, &mut app, rx).await;
    ratatui::restore();
    // `tx` lives until here so the event channel never closes under the loop.
    drop(tx);
    result
}

async fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, mut rx: mpsc::Receiver<AppEvent>) -> Result<()> {
    let logs = app.logs.clone();
    let mut events = crossterm::event::EventStream::new();
    // Clock, "ago" labels and staleness need a redraw even when idle.
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        terminal.draw(|f| draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(e)) => app.on_terminal_event(e),
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(event) = rx.recv() => {
                app.on_app_event(event);
                while let Ok(event) = rx.try_recv() {
                    app.on_app_event(event);
                }
            }
            () = logs.notify.notified() => {}
            _ = tick.tick() => {}
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
    let [top, body, log] =
        Rect::new(area.x, area.y, area.width, area.height).layout(&ratatui::layout::Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(log_height),
        ]));
    let buf = frame.buffer_mut();
    chrome::top_bar(app, top, buf);
    match app.view {
        View::Dashboard => dashboard(app, body, buf),
        View::Detail => detail::render(app, body, buf),
    }
    if app.log_open {
        chrome::log_panel(app, log, buf);
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
mod tests {
    use super::*;
    use crate::state::{DeviceInfo, IcdInfo, Metric, Transport, unix_minute};
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
    fn fake_app() -> App {
        let config = Config::parse(CONFIG).unwrap();
        let mut app = App::new(config, LogBuffer::new(), Theme { truecolor: true, ascii: false });
        let now = unix_minute();
        let live = Status::Live { max_interval_s: 60 };
        let info = |transport, icd| DeviceInfo {
            node_id: 2,
            vendor: Some("Acme".into()),
            product: Some("Gadget".into()),
            firmware: Some("1.0".into()),
            transport,
            endpoint: 1,
            icd,
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
        app.devices[0].info = Some(info(Transport::WiFi, None));
        set(&mut app, 0, Metric::On, &|k| if k < 25 { 1.0 } else { 0.0 });
        app.devices[1].info = Some(info(Transport::Thread, None));
        set(&mut app, 1, Metric::Temperature, &|k| 19.0 + (60 - k) as f64 * 0.025);
        set(&mut app, 1, Metric::HeatSetpoint, &|_| 21.0);
        set(&mut app, 1, Metric::Running, &|k| if k < 40 { 1.0 } else { 0.0 });
        set(&mut app, 1, Metric::Mode, &|_| 4.0);
        set(&mut app, 2, Metric::Temperature, &|k| 20.5 + ((k as f64) / 9.0).sin() * 0.6);
        set(&mut app, 2, Metric::Humidity, &|_| 48.0);
        app.devices[3].info = Some(info(Transport::Thread, Some(IcdInfo { long_idle_time: false, check_in: true, idle_mode_s: Some(300) })));
        set(&mut app, 3, Metric::Temperature, &|k| 17.2 + (k as f64) * 0.01);
        set(&mut app, 3, Metric::Humidity, &|_| 55.0);
        set(&mut app, 3, Metric::Battery, &|_| 15.0);
        set(&mut app, 4, Metric::On, &|_| 1.0);
        app.devices[5].status = Status::Offline { error: "no route".into() };
        app.selected = 1;
        app
    }

    fn render(app: &mut App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        let buf = term.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Strip the volatile clock so snapshots are stable.
    fn stable(s: String) -> String {
        let mut lines: Vec<String> = s.lines().map(str::to_string).collect();
        // The top bar ends with the clock, "HH:MM".
        if let Some(top) = lines.first_mut()
            && let Some((cut, _)) = top.char_indices().rev().nth(4)
        {
            top.replace_range(cut.., "HH:MM");
        }
        lines.join("\n")
    }

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

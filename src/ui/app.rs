//! UI state and input handling. Rendering lives in `widgets`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use super::layout::{self, Layout, RoomSpec, TileSpec};
use super::motion::{self, Ticker, Tone, Tween};
use super::theme::Theme;
use crate::config::{Config, LogPanel};
use crate::logbuf::LogBuffer;
use crate::config::DeviceKind;
use crate::state::{AppEvent, DeviceState, DeviceUpdate, Metric, Status};

/// Chart/sparkline ranges in minutes.
const RANGES: &[(i64, &str)] = &[
    (15, "15m"),
    (60, "1h"),
    (6 * 60, "6h"),
    (24 * 60, "24h"),
    (7 * 24 * 60, "7d"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Detail,
}

pub struct App {
    pub config: Config,
    /// Same order as `config.devices`.
    pub devices: Vec<DeviceState>,
    /// For each room (config order), its device indices (config order).
    pub rooms: Vec<Vec<usize>>,
    pub theme: Theme,
    pub logs: Arc<LogBuffer>,
    pub view: View,
    pub selected: usize,
    range: usize,
    pub log_open: bool,
    pub log_focus: bool,
    /// Lines scrolled up from the newest; 0 = following.
    pub log_scroll: usize,
    seen_warnings: u64,
    pub show_help: bool,
    /// Dashboard vertical scroll, when the layout is taller than the screen.
    pub scroll: u16,
    /// Most recent layout, in dashboard-content coordinates.
    pub layout: Layout,
    pub quit: bool,
    /// Pair/unpair dialog, if open.
    pub modal: Option<Modal>,
    /// Work for the event loop to start (it owns the controller).
    pub action: Option<Action>,
    /// Attestation roots are loaded, so pairing can work.
    pub can_pair: bool,
    /// Animations on (config `ui.motion`, toggled with `m`).
    pub motion: bool,
    /// When the dashboard first appeared (startup sweep).
    pub epoch: Instant,
    tweens: HashMap<(usize, Metric), Tween>,
    /// Last time each device reported, for the update flash.
    flashes: HashMap<usize, Instant>,
    pub ticker: Ticker,
    /// The ticker overflowed its line last frame and is scrolling.
    pub ticker_scrolling: bool,
    /// Last temperature announced in the ticker, per device.
    announced: HashMap<usize, f64>,
    /// Devices that went offline after being live (for "back online").
    lost: std::collections::HashSet<usize>,
}

#[derive(Debug)]
pub struct Modal {
    pub kind: ModalKind,
    pub device: usize,
    /// Code being typed (pairing only).
    pub input: String,
    pub phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Pair,
    Unpair,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Typing a code / confirming. `error` is from the last attempt.
    Input { error: Option<String> },
    Working { since: Instant },
    Done { ok: bool, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Pair { device: usize, code: String },
    Unpair { device: usize },
}

impl App {
    pub fn new(config: Config, logs: Arc<LogBuffer>, theme: Theme) -> Self {
        let devices = config
            .devices
            .iter()
            .map(|_| DeviceState::new(config.ui.history_hours))
            .collect();
        let rooms = config
            .rooms
            .iter()
            .map(|r| {
                config
                    .devices
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| d.room.get_ref() == r.id.get_ref())
                    .map(|(i, _)| i)
                    .collect()
            })
            .collect();
        let log_open = config.log.panel == LogPanel::Open;
        let motion = config.ui.motion;
        let seen_warnings = logs.warnings();
        // Default to the 1h range, or the longest one history allows.
        let range = 1.min(max_range_index(config.ui.history_hours));
        Self {
            config,
            devices,
            rooms,
            theme,
            logs,
            view: View::Dashboard,
            selected: 0,
            range,
            log_open,
            log_focus: false,
            log_scroll: 0,
            seen_warnings,
            show_help: false,
            scroll: 0,
            layout: Layout::default(),
            quit: false,
            modal: None,
            action: None,
            can_pair: false,
            motion,
            epoch: Instant::now(),
            tweens: HashMap::new(),
            flashes: HashMap::new(),
            ticker: Ticker::default(),
            ticker_scrolling: false,
            announced: HashMap::new(),
            lost: Default::default(),
        }
    }

    pub fn room_specs(&self) -> Vec<RoomSpec> {
        self.rooms
            .iter()
            .map(|devs| RoomSpec {
                tiles: devs
                    .iter()
                    .map(|&i| TileSpec {
                        kind: self.config.devices[i].kind,
                        featured: self.config.devices[i].featured,
                    })
                    .collect(),
            })
            .collect()
    }

    /// Recompute the layout for a dashboard body of `size`, and keep the
    /// selected tile scrolled into view.
    pub fn relayout(&mut self, size: Rect) {
        self.layout = layout::layout(&self.room_specs(), Rect::new(0, 0, size.width, size.height));
        let max_scroll = self.layout.height.saturating_sub(size.height);
        if let Some(r) = self.selected_rect() {
            if r.y < self.scroll {
                self.scroll = r.y.saturating_sub(1);
            } else if r.bottom() > self.scroll + size.height {
                self.scroll = (r.bottom() + 1).saturating_sub(size.height);
            }
        }
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn device_at(&self, room: usize, tile: usize) -> usize {
        self.rooms[room][tile]
    }

    fn selected_rect(&self) -> Option<Rect> {
        self.layout
            .tiles
            .iter()
            .find(|t| self.device_at(t.room, t.tile) == self.selected)
            .map(|t| t.rect)
    }

    pub fn range_minutes(&self) -> i64 {
        RANGES[self.range].0
    }

    pub fn range_label(&self) -> &'static str {
        RANGES[self.range].1
    }

    pub fn ranges(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        RANGES[..=max_range_index(self.config.ui.history_hours)]
            .iter()
            .enumerate()
            .map(|(i, (_, label))| (*label, i == self.range))
    }

    pub fn unseen_warnings(&self) -> u64 {
        self.logs.warnings().saturating_sub(self.seen_warnings)
    }

    pub fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Device { id, update } => {
                if let Some(i) = self.device_index(&id) {
                    self.note_update(i, &update);
                    self.devices[i].apply(update);
                }
            }
            AppEvent::Paired { id, result } => {
                let Some(i) = self.device_index(&id) else { return };
                let (ok, message) = match result {
                    Ok(node_id) => {
                        self.devices[i].apply(DeviceUpdate::Status(Status::Connecting));
                        let name = self.config.devices[i].name.clone();
                        self.ticker.push(i, format!("{name} paired"), Tone::Good);
                        self.flashes.insert(i, Instant::now());
                        (true, format!("Paired as node 0x{node_id:X}. Connecting…"))
                    }
                    Err(e) => (false, e),
                };
                self.finish_modal(i, ModalKind::Pair, ok, message);
            }
            AppEvent::Unpaired { id, result } => {
                let Some(i) = self.device_index(&id) else { return };
                let (ok, message) = match result {
                    Ok(note) => {
                        self.devices[i] = DeviceState::new(self.config.ui.history_hours);
                        (true, note)
                    }
                    Err(e) => (false, e),
                };
                self.finish_modal(i, ModalKind::Unpair, ok, message);
            }
        }
    }

    fn device_index(&self, id: &str) -> Option<usize> {
        self.config.devices.iter().position(|d| d.id.get_ref() == id)
    }

    fn finish_modal(&mut self, device: usize, kind: ModalKind, ok: bool, message: String) {
        match &mut self.modal {
            Some(m) if m.device == device && m.kind == kind => {
                if ok {
                    m.phase = Phase::Done { ok, message };
                } else {
                    // Let the user fix the code and retry without retyping.
                    m.phase = if kind == ModalKind::Pair {
                        Phase::Input { error: Some(message) }
                    } else {
                        Phase::Done { ok, message }
                    };
                }
            }
            // Dialog was dismissed while working; the log has the outcome.
            _ => {}
        }
    }

    /// Something is moving on screen (needs fast redraws).
    pub fn animating(&self) -> bool {
        if matches!(self.modal, Some(Modal { phase: Phase::Working { .. }, .. })) {
            return true;
        }
        if !self.motion {
            return false;
        }
        let now = Instant::now();
        let sweep_end = self.epoch + motion::SWEEP_STAGGER * self.rooms.len() as u32 + motion::SWEEP;
        now < sweep_end
            || self.tweens.values().any(|t| !t.done(now))
            || self.flashes.values().any(|&f| now.saturating_duration_since(f) < motion::FLASH)
            || self.devices.iter().any(|d| d.status == Status::Connecting)
    }

    /// Slow ambient motion (breathing lights, heating) wants a modest
    /// frame rate even when nothing else is happening.
    pub fn ambient(&self) -> bool {
        self.motion
            && self.config.devices.iter().zip(&self.devices).any(|(c, d)| {
                let on = |m| d.current.get(&m).copied().flatten().is_some_and(|v| v != 0.0);
                (c.kind == DeviceKind::Plug && on(Metric::On)) || on(Metric::Running)
            })
    }

    /// The value to display for a metric: mid-tween if animating.
    pub fn shown(&self, device: usize, metric: Metric) -> Option<f64> {
        let v = self.devices[device].current.get(&metric).copied().flatten();
        match self.tweens.get(&(device, metric)) {
            Some(t) if self.motion && v.is_some() => Some(t.value(Instant::now())),
            _ => v,
        }
    }

    /// 1 → 0 over the flash after a device reports.
    pub fn flash(&self, device: usize) -> f64 {
        match self.flashes.get(&device) {
            Some(&at) if self.motion => 1.0 - motion::progress(at, motion::FLASH, Instant::now()),
            _ => 0.0,
        }
    }

    /// 0 → 1 reveal of a room during the startup sweep.
    pub fn reveal(&self, room: usize) -> f64 {
        if !self.motion {
            return 1.0;
        }
        let start = self.epoch + motion::SWEEP_STAGGER * room as u32;
        motion::ease_out_cubic(motion::progress(start, motion::SWEEP, Instant::now()))
    }

    /// Start tweens/flashes and record ticker events for an incoming update.
    fn note_update(&mut self, i: usize, update: &DeviceUpdate) {
        let now = Instant::now();
        let name = self.config.devices[i].name.clone();
        let d = &self.devices[i];
        match update {
            DeviceUpdate::Reading { metric, value } => {
                let old = d.current.get(metric).copied().flatten();
                self.flashes.insert(i, now);
                if let (Some(new), Some(old)) = (value, old)
                    && (new - old).abs() > f64::EPSILON
                {
                    // Tween from wherever the display currently is.
                    let from = self.shown(i, *metric).unwrap_or(old);
                    self.tweens.insert((i, *metric), Tween { from, to: *new, start: now });
                }
                match (metric, value, old) {
                    (Metric::On, Some(v), Some(o)) if (v != &0.0) != (o != 0.0) => {
                        let on = *v != 0.0;
                        self.ticker.push(i, format!("{name} {}", if on { "on" } else { "off" }), if on { Tone::Glow } else { Tone::Normal });
                    }
                    (Metric::Running, Some(v), Some(o)) if (*v as u64 & 1) != (o as u64 & 1) => {
                        let heating = *v as u64 & 1 != 0;
                        self.ticker.push(i, format!("{name} heating {}", if heating { "on" } else { "off" }), if heating { Tone::Heat } else { Tone::Normal });
                    }
                    (Metric::Temperature, Some(v), _) => {
                        // Announce moves of half a degree or more, not every wobble.
                        match self.announced.get(&i).copied() {
                            None => {
                                self.announced.insert(i, *v);
                            }
                            Some(last) if (v - last).abs() >= 0.5 => {
                                let arrow = if *v > last { "↑" } else { "↓" };
                                let text = format!("{name} {} {arrow}", super::widgets::fmt_temp_unit(self.config.ui.temperature_unit, *v));
                                self.ticker.push(i, text, Tone::Normal);
                                self.announced.insert(i, *v);
                            }
                            Some(_) => {}
                        }
                    }
                    _ => {}
                }
            }
            DeviceUpdate::Status(new) => match (&d.status, new) {
                (Status::Live { .. } | Status::Reconnecting { .. }, Status::Offline { .. }) => {
                    self.ticker.push(i, format!("{name} offline"), Tone::Bad);
                    self.lost.insert(i);
                }
                (_, Status::Live { .. }) if self.lost.remove(&i) => {
                    self.ticker.push(i, format!("{name} back online"), Tone::Good);
                }
                _ => {}
            },
            DeviceUpdate::Info(_) => {}
        }
        // Drop finished animations so `animating()` goes quiet.
        self.tweens.retain(|_, t| !t.done(now));
        self.flashes.retain(|_, &mut at| now.saturating_duration_since(at) < motion::FLASH);
    }

    fn open_modal(&mut self, kind: ModalKind) {
        if self.devices.is_empty() {
            return;
        }
        let device = self.selected;
        let paired = self.health(device) != Health::Unpaired;
        let id = self.config.devices[device].id.get_ref().clone();
        let phase = match (kind, paired) {
            (ModalKind::Pair, true) => Phase::Done {
                ok: false,
                message: format!("`{id}` is already paired. Press U to unpair it first."),
            },
            (ModalKind::Unpair, false) => Phase::Done { ok: false, message: format!("`{id}` isn't paired.") },
            (ModalKind::Pair, false) if !self.can_pair => Phase::Done {
                ok: false,
                message: "Attestation roots are missing. Quit, run `mattertui fetch-roots`, then restart.".into(),
            },
            _ => Phase::Input { error: None },
        };
        self.modal = Some(Modal { kind, device, input: String::new(), phase });
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(m) = &mut self.modal else { return };
        match (&m.phase, key.code) {
            // Closing never cancels work in flight; the outcome still lands in the log.
            (_, KeyCode::Esc) => self.modal = None,
            (Phase::Done { .. }, KeyCode::Enter) => self.modal = None,
            (Phase::Input { .. }, KeyCode::Backspace) if m.kind == ModalKind::Pair => {
                m.input.pop();
                m.phase = Phase::Input { error: None };
            }
            (Phase::Input { .. }, KeyCode::Char(c)) if m.kind == ModalKind::Pair => {
                // Digits, separators, and the QR alphabet (base38: A-Z 0-9 - .) plus "MT:".
                if (c.is_ascii_alphanumeric() || matches!(c, '-' | ' ' | '.' | ':')) && m.input.len() < 40 {
                    m.input.push(c.to_ascii_uppercase());
                    m.phase = Phase::Input { error: None };
                }
            }
            (Phase::Input { .. }, KeyCode::Enter) if m.kind == ModalKind::Pair => {
                match crate::matter::pairing::normalize_code(&m.input) {
                    Ok(code) => {
                        self.action = Some(Action::Pair { device: m.device, code });
                        m.phase = Phase::Working { since: Instant::now() };
                    }
                    Err(e) => m.phase = Phase::Input { error: Some(e.to_string()) },
                }
            }
            (Phase::Input { .. }, KeyCode::Char('y' | 'Y')) if m.kind == ModalKind::Unpair => {
                self.action = Some(Action::Unpair { device: m.device });
                m.phase = Phase::Working { since: Instant::now() };
            }
            (Phase::Input { .. }, KeyCode::Char('n' | 'N')) if m.kind == ModalKind::Unpair => self.modal = None,
            _ => {}
        }
    }

    pub fn on_terminal_event(&mut self, event: Event) {
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
        {
            self.on_key(key);
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.modal.is_some() {
            self.on_modal_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('p') => self.open_modal(ModalKind::Pair),
            KeyCode::Char('U') => self.open_modal(ModalKind::Unpair),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('m') => {
                self.motion = !self.motion;
                tracing::info!("motion {}", if self.motion { "on" } else { "off" });
            }
            KeyCode::Esc if self.show_help => self.show_help = false,
            KeyCode::Char('`') => {
                self.log_open = !self.log_open;
                self.log_focus = false;
                self.seen_warnings = self.logs.warnings();
            }
            KeyCode::Tab if self.log_open => self.log_focus = !self.log_focus,
            _ if self.log_focus => self.on_log_key(key.code),
            KeyCode::Char('[') => self.range = self.range.saturating_sub(1),
            KeyCode::Char(']') => {
                self.range = (self.range + 1).min(max_range_index(self.config.ui.history_hours))
            }
            KeyCode::Enter if !self.devices.is_empty() => {
                self.view = match self.view {
                    View::Dashboard => View::Detail,
                    View::Detail => View::Dashboard,
                }
            }
            KeyCode::Esc => self.view = View::Dashboard,
            _ => {
                if let Some(dir) = direction(key.code) {
                    match self.view {
                        View::Dashboard => self.move_selection(dir),
                        View::Detail => self.cycle_selection(dir),
                    }
                }
            }
        }
        if self.log_open {
            // Opening the panel (or using it) counts as having seen warnings.
            self.seen_warnings = self.logs.warnings();
        }
    }

    fn on_log_key(&mut self, code: KeyCode) {
        let total = self.logs.snapshot().len();
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.log_scroll += 1,
            KeyCode::Down | KeyCode::Char('j') => self.log_scroll = self.log_scroll.saturating_sub(1),
            KeyCode::PageUp => self.log_scroll += 10,
            KeyCode::PageDown => self.log_scroll = self.log_scroll.saturating_sub(10),
            KeyCode::End => self.log_scroll = 0,
            KeyCode::Home => self.log_scroll = total,
            KeyCode::Esc => self.log_focus = false,
            _ => {}
        }
        self.log_scroll = self.log_scroll.min(total);
    }

    /// Move to the nearest tile in `dir`, by tile centres.
    fn move_selection(&mut self, dir: Dir) {
        let Some(from) = self.selected_rect() else {
            return;
        };
        let (fx, fy) = centre(from);
        let best = self
            .layout
            .tiles
            .iter()
            .filter_map(|t| {
                let (x, y) = centre(t.rect);
                let (dx, dy) = (x - fx, y - fy);
                // Primary distance along `dir` (must be positive), plus the
                // sideways offset weighted so straight-ahead wins.
                let (along, across) = match dir {
                    Dir::Left => (-dx, dy),
                    Dir::Right => (dx, dy),
                    Dir::Up => (-dy, dx),
                    Dir::Down => (dy, dx),
                };
                (along > 0).then(|| (along + 3 * across.abs(), self.device_at(t.room, t.tile)))
            })
            .min_by_key(|&(score, _)| score);
        if let Some((_, device)) = best {
            self.selected = device;
        }
    }

    /// In the detail view, arrows step through devices in dashboard order.
    fn cycle_selection(&mut self, dir: Dir) {
        let order: Vec<usize> = self.rooms.iter().flatten().copied().collect();
        let Some(pos) = order.iter().position(|&d| d == self.selected) else {
            return;
        };
        let n = order.len();
        self.selected = match dir {
            Dir::Left | Dir::Up => order[(pos + n - 1) % n],
            Dir::Right | Dir::Down => order[(pos + 1) % n],
        };
    }

    /// Health of a device right now, for colouring and badges.
    pub fn health(&self, device: usize) -> Health {
        let d = &self.devices[device];
        match &d.status {
            Status::Unpaired => Health::Unpaired,
            Status::Connecting => Health::Connecting,
            // Resubscribes are routine (sleepy devices, Wi-Fi blips) and the
            // last values are still good, so only escalate if it drags on.
            Status::Reconnecting { .. } if d.status_since.elapsed() < RECONNECT_GRACE => Health::Reconnecting,
            Status::Reconnecting { .. } => Health::Stale,
            Status::Offline { .. } => Health::Offline,
            Status::Live { max_interval_s } => {
                let allowed = Duration::from_secs(u64::from(*max_interval_s) * 2 + 30);
                match d.last_report {
                    Some(t) if t.elapsed() > allowed => Health::Stale,
                    _ => Health::Live,
                }
            }
        }
    }
}

/// How long a reconnect stays low-key before the tile shows it as stale.
const RECONNECT_GRACE: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Unpaired,
    Connecting,
    Live,
    Stale,
    /// Briefly re-establishing the subscription; values are still current.
    Reconnecting,
    Offline,
}

impl Health {
    /// Live, or close enough that values should look normal.
    pub fn is_ok(self) -> bool {
        matches!(self, Health::Live | Health::Reconnecting)
    }
}

#[derive(Debug, Clone, Copy)]
enum Dir {
    Left,
    Right,
    Up,
    Down,
}

fn direction(code: KeyCode) -> Option<Dir> {
    Some(match code {
        KeyCode::Left | KeyCode::Char('h') => Dir::Left,
        KeyCode::Right | KeyCode::Char('l') => Dir::Right,
        KeyCode::Up | KeyCode::Char('k') => Dir::Up,
        KeyCode::Down | KeyCode::Char('j') => Dir::Down,
        _ => return None,
    })
}

fn centre(r: Rect) -> (i32, i32) {
    (
        i32::from(r.x) + i32::from(r.width) / 2,
        i32::from(r.y) + i32::from(r.height) / 2,
    )
}

fn max_range_index(history_hours: u32) -> usize {
    let max_min = i64::from(history_hours) * 60;
    RANGES
        .iter()
        .rposition(|(m, _)| *m <= max_min)
        .unwrap_or(0)
}

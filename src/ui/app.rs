//! UI state and input handling. Rendering lives in `widgets`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use super::layout::{self, Layout, RoomSpec, TileSpec};
use super::theme::Theme;
use crate::config::{Config, LogPanel};
use crate::logbuf::LogBuffer;
use crate::state::{AppEvent, DeviceState, Status};

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
                    self.devices[i].apply(update);
                }
            }
            AppEvent::Paired { id, result } => {
                let Some(i) = self.device_index(&id) else { return };
                let (ok, message) = match result {
                    Ok(node_id) => {
                        self.devices[i].apply(crate::state::DeviceUpdate::Status(Status::Connecting));
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
        matches!(self.modal, Some(Modal { phase: Phase::Working { .. }, .. }))
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

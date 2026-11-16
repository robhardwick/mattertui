//! Device tiles: borderless sections inside a room's box.
//!
//! ```text
//! ▸ 🌡  Bedside ───────────────────────── 12s   header (name, rule, status)
//!   🌡  17.2° ↓        💧  55%       🔋  15%   readouts on a fixed grid
//!   ⣀⣠⣤⣴⣶⣾⣿⣿⣿⣷⣶⣦⣤⣄⣀              21.0°   gradient graph, grows with
//!   ⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣶⣤⣀         17.0°   spare rows
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use tui_big_text::{BigText, PixelSize};

use super::{current, fmt_temp, health_badge, metric_graph, timeline, trend};
use crate::config::DeviceKind;
use crate::state::{Metric, unix_minute};
use crate::ui::app::{App, Health};
use crate::ui::graph;
use crate::ui::icons;
use crate::ui::layout::Variant;
use crate::ui::motion::mix;

pub fn render(app: &App, device: usize, variant: Variant, accent: Color, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    if variant == Variant::Compact {
        compact(app, device, accent, area, buf);
        return;
    }
    header(app, device, accent, row(area, 0), buf);
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
    if app.health(device) == Health::Unpaired {
        Line::from(vec![
            Span::styled("not paired · ", app.theme.muted()),
            Span::styled("p", app.theme.bold(app.theme.orange())),
            Span::styled(" to pair", app.theme.muted()),
        ])
        .render(row(body, 0), buf);
        return;
    }
    match (variant, app.config.devices[device].kind) {
        (_, DeviceKind::Plug) => plug(app, device, body, buf),
        (Variant::Featured, kind) => featured(app, device, kind, body, buf),
        (_, DeviceKind::TemperatureSensor) => sensor(app, device, body, buf),
        (_, DeviceKind::Thermostat) => thermostat(app, device, body, buf),
    }
}

fn row(area: Rect, n: u16) -> Rect {
    Rect::new(area.x, area.y + n, area.width, 1).intersection(area)
}

fn rows_from(area: Rect, n: u16) -> Rect {
    Rect::new(area.x, area.y + n, area.width, area.height.saturating_sub(n))
}

fn icon(app: &App, device: usize) -> String {
    let cfg = &app.config.devices[device];
    icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), app.theme.ascii)
}

/// `▸ 🌡  Bedside ───────────── 12s`: name, a rule to the right edge, and
/// the device's status (or how long ago it last reported).
fn header(app: &App, device: usize, accent: Color, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let selected = device == app.selected;
    let name = &app.config.devices[device].name;
    let (marker, name_style) = if selected {
        ("▸ ", t.bold(t.orange()))
    } else {
        ("  ", t.bold(accent))
    };
    let left = Line::from(vec![
        Span::styled(marker, Style::new().fg(t.orange())),
        Span::raw(icon(app, device)),
        Span::raw(" "),
        Span::styled(name.clone(), name_style),
        Span::raw(" "),
    ]);
    let (status, status_colour) = match health_badge(app, app.health(device)) {
        Some(b) => b,
        None => (app.devices[device].last_report.map(super::ago).unwrap_or_default(), t.faint()),
    };
    let right = Line::styled(format!(" {status}"), Style::new().fg(status_colour));

    // The rule flashes in the room tint when the device reports.
    let rule = mix(t.faint(), accent, app.flash(device) * 0.8);
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_char(if t.ascii { '-' } else { '─' }).set_style(Style::new().fg(rule));
    }
    let lw = left.width() as u16;
    left.render(area, buf);
    if (right.width() as u16) + lw < area.width {
        right.alignment(Alignment::Right).render(area, buf);
    }
}

/// Values are dimmed when the device isn't live, so stale data looks stale.
fn value_style(app: &App, device: usize, colour: Color) -> Style {
    if app.health(device).is_ok() {
        Style::new().fg(colour).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(app.theme.dim())
    }
}

/// Lay spans out on a fixed grid: each field starts at its column.
fn fields(area: Rect, buf: &mut Buffer, fields: Vec<(u16, Option<Line<'static>>)>) {
    for (col, line) in fields {
        if let Some(line) = line
            && col < area.width
        {
            line.render(Rect::new(area.x + col, area.y, area.width - col, 1), buf);
        }
    }
}

// --- Readout pieces -------------------------------------------------------

fn temperature(app: &App, device: usize) -> Line<'static> {
    let t = &app.theme;
    let d = &app.devices[device];
    let glyph = format!("{} ", icons::cell("thermometer", t.ascii));
    match app.shown(device, Metric::Temperature) {
        Some(c) => Line::from(vec![
            Span::styled(glyph, Style::new().fg(t.temperature(c))),
            Span::styled(fmt_temp(app, c), value_style(app, device, t.temperature(c))),
            Span::styled(format!(" {}", trend(d.history.get(&Metric::Temperature), c, unix_minute())), t.muted()),
        ]),
        None => Line::from(vec![Span::styled(glyph, t.muted()), Span::styled("—", t.muted())]),
    }
}

fn humidity(app: &App, device: usize) -> Option<Line<'static>> {
    let t = &app.theme;
    let h = app.shown(device, Metric::Humidity)?;
    Some(Line::from(vec![
        Span::styled(format!("{} ", icons::cell("humidity", t.ascii)), Style::new().fg(t.humidity(h))),
        Span::styled(format!("{h:.0}%"), value_style(app, device, t.humidity(h))),
    ]))
}

fn battery(app: &App, device: usize) -> Option<Line<'static>> {
    let t = &app.theme;
    let d = &app.devices[device];
    let icon = format!("{} ", icons::cell("battery", t.ascii));
    if let Some(b) = current(d, Metric::Battery) {
        let colour = if b < 20.0 { t.bad() } else { t.dim() };
        return Some(Line::styled(format!("{icon}{b:.0}%"), Style::new().fg(colour)));
    }
    // Devices without a percentage report a charge level instead.
    let (text, colour) = match current(d, Metric::BatteryLevel)? as u8 {
        0 => ("ok", t.dim()),
        1 => ("low", t.warn()),
        _ => ("replace", t.bad()),
    };
    Some(Line::styled(format!("{icon}{text}"), Style::new().fg(colour)))
}

/// Thermostat facts, if discovery has run.
fn thermostat_info(app: &App, device: usize) -> Option<crate::state::ThermostatInfo> {
    app.devices[device].info.as_ref().and_then(|i| i.thermostat)
}

fn mode(app: &App, device: usize) -> Option<u8> {
    current(&app.devices[device], Metric::Mode).map(|m| m as u8)
}

/// `→ 18.0°` (and the cooling setpoint when relevant), or `off`.
fn setpoint(app: &App, device: usize) -> Line<'static> {
    let t = &app.theme;
    let mode = mode(app, device);
    let (can_heat, can_cool) = thermostat_info(app, device).map_or((true, true), |c| (c.can_heat, c.can_cool));
    if mode == Some(0) {
        return Line::styled("off", t.muted());
    }
    let mut spans = vec![];
    if let Some(sp) = app.shown(device, Metric::HeatSetpoint)
        && can_heat
        && mode != Some(3)
    {
        spans.push(Span::styled("→ ", Style::new().fg(t.heat())));
        spans.push(Span::styled(fmt_temp(app, sp), value_style(app, device, t.heat())));
    }
    if let Some(sp) = app.shown(device, Metric::CoolSetpoint)
        && can_cool
        && matches!(mode, Some(1 | 3))
    {
        spans.push(Span::styled(format!("  {} ", icons::cell("cool", t.ascii)), Style::new().fg(t.cool())));
        spans.push(Span::styled(fmt_temp(app, sp), value_style(app, device, t.cool())));
    }
    // "heat" on a heating-only device says nothing; other modes do.
    if let Some(m) = mode
        && !(m == 4 && !can_cool)
    {
        spans.push(Span::styled(format!("  {}", system_mode(m)), t.muted()));
    }
    Line::from(spans)
}

fn running(app: &App, device: usize) -> Option<Line<'static>> {
    let t = &app.theme;
    if mode(app, device) == Some(0) {
        return None;
    }
    let d = &app.devices[device];
    let running = current(d, Metric::Running).map_or(0, |r| r as u64);
    let demand = current(d, Metric::HeatDemand);
    let (mut text, colour) = if running & 0b001 != 0 || demand.is_some_and(|p| p > 0.0) {
        (format!("{} heating", icons::cell("fire", t.ascii)), super::flame(app))
    } else if running & 0b010 != 0 {
        (format!("{} cooling", icons::cell("cool", t.ascii)), t.cool())
    } else {
        ("idle".to_string(), t.dim())
    };
    if let Some(p) = demand.filter(|p| *p > 0.0) {
        text.push_str(&format!(" {p:.0}%"));
    }
    Some(Line::styled(text, Style::new().fg(colour)))
}

// --- Tiles ----------------------------------------------------------------

fn sensor(app: &App, device: usize, body: Rect, buf: &mut Buffer) {
    let w = body.width;
    fields(
        row(body, 0),
        buf,
        vec![
            (0, Some(temperature(app, device))),
            (w * 2 / 5, humidity(app, device)),
        ],
    );
    if let Some(b) = battery(app, device) {
        b.alignment(Alignment::Right).render(row(body, 0), buf);
    }
    metric_graph(app, device, Metric::Temperature, None, rows_from(body, 1), buf);
}

fn thermostat(app: &App, device: usize, body: Rect, buf: &mut Buffer) {
    let w = body.width;
    fields(
        row(body, 0),
        buf,
        vec![(0, Some(temperature(app, device))), (w * 2 / 5, humidity(app, device))],
    );
    if let Some(b) = battery(app, device) {
        b.alignment(Alignment::Right).render(row(body, 0), buf);
    }
    fields(row(body, 1), buf, vec![(0, Some(setpoint(app, device))), (w * 2 / 5, running(app, device))]);
    metric_graph(app, device, Metric::Temperature, Some(Metric::HeatSetpoint), rows_from(body, 2), buf);
}

fn plug(app: &App, device: usize, body: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    let on = current(d, Metric::On);
    let (dot, text, colour) = match on {
        Some(v) if v != 0.0 => ("●", "ON", super::glow(app)),
        Some(_) => ("○", "off", t.dim()),
        None => ("○", "—", t.dim()),
    };
    let mut spans = vec![
        Span::styled(format!("{dot} "), Style::new().fg(colour)),
        Span::styled(text, value_style(app, device, colour)),
    ];
    if let Some(since) = on_since(app, device) {
        spans.push(Span::styled(format!("  for {since}"), t.muted()));
    }
    Line::from(spans).render(row(body, 0), buf);
    if let Some(info) = &d.info {
        Line::styled(transport_name(info.transport), Style::new().fg(t.faint()))
            .alignment(Alignment::Right)
            .render(row(body, 0), buf);
    }
    // One row: a timeline. More: a square-wave graph of on/off.
    if body.height <= 2 {
        timeline(app, d.history.get(&Metric::On), row(body, 1), buf, super::glow(app));
    } else {
        super::metric_graph_bare(app, device, Metric::On, None, rows_from(body, 1), buf);
    }
}

/// How long a plug has been in its current state, from history.
fn on_since(app: &App, device: usize) -> Option<String> {
    let d = &app.devices[device];
    let now_v = current(d, Metric::On)?;
    let h = d.history.get(&Metric::On)?;
    let changed = h.buckets().filter(|b| b.last != now_v).last()?;
    let mins = unix_minute() - changed.minute;
    Some(match mins {
        ..1 => "<1m".into(),
        1..60 => format!("{mins}m"),
        _ => format!("{}h {:02}m", mins / 60, mins % 60),
    })
}

fn featured(app: &App, device: usize, kind: DeviceKind, body: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];

    // Big readout on the left, details on a grid to its right.
    let temp = app.shown(device, Metric::Temperature);
    let big = temp.map_or("--.-".into(), |c| fmt_temp(app, c).trim_end_matches('°').to_string());
    let colour = temp.map_or(t.dim(), |c| t.temperature(c));
    let big_w = (big.chars().count() as u16) * 4;
    let big_area = Rect::new(body.x, body.y, big_w.min(body.width), 4).intersection(body);
    BigText::builder()
        .pixel_size(PixelSize::Quadrant)
        .style(value_style(app, device, colour))
        .lines(vec![big.into()])
        .build()
        .render(big_area, buf);

    let side = Rect::new(big_area.right() + 2, body.y, body.width.saturating_sub(big_w + 2), 4);
    let unit = match app.config.ui.temperature_unit {
        crate::config::TemperatureUnit::Celsius => "°C",
        crate::config::TemperatureUnit::Fahrenheit => "°F",
    };
    let trend = temp.map_or("", |c| trend(d.history.get(&Metric::Temperature), c, unix_minute()));
    Line::from(vec![
        Span::styled(unit, Style::new().fg(colour)),
        Span::styled(format!(" {trend}"), t.muted()),
    ])
    .render(row(side, 0), buf);
    if let Some(h) = humidity(app, device) {
        h.alignment(Alignment::Right).render(row(side, 0), buf);
    }
    if kind == DeviceKind::Thermostat {
        setpoint(app, device).render(row(side, 1), buf);
        if let Some(r) = running(app, device) {
            r.render(row(side, 2), buf);
        }
    }
    if let Some(b) = battery(app, device) {
        b.render(row(side, 3), buf);
    }

    // A gauge (thermostats) or humidity meter (sensors), then the graph.
    let gauge_row = row(body, 4);
    match kind {
        DeviceKind::Thermostat => setpoint_gauge(app, device, gauge_row, buf),
        _ => humidity_meter(app, device, gauge_row, buf),
    }
    let overlay = (kind == DeviceKind::Thermostat).then_some(Metric::HeatSetpoint);
    metric_graph(app, device, Metric::Temperature, overlay, rows_from(body, 5), buf);
}

/// Where the temperature and setpoint sit between the setpoint limits:
/// `5° ■■■■■■■■●◆■■■■■■■■■ 30°`.
fn setpoint_gauge(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let (lo, hi) = thermostat_info(app, device).and_then(|i| i.heat_limits).unwrap_or((5.0, 30.0));
    let lo_label = fmt_temp(app, lo).replace(".0°", "°");
    let hi_label = fmt_temp(app, hi).replace(".0°", "°");
    let track_x = area.x + lo_label.chars().count() as u16 + 1;
    let track_w = area.width.saturating_sub(lo_label.chars().count() as u16 + hi_label.chars().count() as u16 + 2);
    if track_w < 6 {
        return;
    }
    Line::styled(lo_label, t.muted()).render(area, buf);
    Line::styled(hi_label, t.muted()).alignment(Alignment::Right).render(area, buf);
    let temp = app.shown(device, Metric::Temperature);
    let frac = temp.map_or(0.0, |c| (c - lo) / (hi - lo));
    let track = Rect::new(track_x, area.y, track_w, 1);
    graph::meter(track, buf, frac, &|p| t.temperature(lo + (hi - lo) * p), t.faint(), t.ascii);

    let col = |v: f64| {
        let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        track_x + (f * f64::from(track_w - 1)).round() as u16
    };
    let (temp_mark, sp_mark) = if t.ascii { ('o', '^') } else { ('●', '◆') };
    let last = track_x + track_w - 1;
    let temp_col = temp.map(col);
    if let Some(c) = temp {
        buf[(col(c), area.y)].set_char(temp_mark).set_style(t.bold(t.fg()));
    }
    if mode(app, device) != Some(0)
        && let Some(sp) = app.shown(device, Metric::HeatSetpoint)
    {
        // Keep both markers visible when they land in the same column.
        let mut x = col(sp);
        if temp_col == Some(x) {
            let above = temp.is_some_and(|c| sp >= c);
            x = if (above && x < last) || x == track_x { x + 1 } else { x - 1 };
        }
        buf[(x, area.y)].set_char(sp_mark).set_style(t.bold(t.heat()));
    }
}

fn humidity_meter(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let Some(h) = app.shown(device, Metric::Humidity) else { return };
    let label = format!("{} ", icons::cell("humidity", t.ascii));
    Line::styled(label.clone(), Style::new().fg(t.humidity(h))).render(area, buf);
    let x = area.x + 3;
    let w = area.width.saturating_sub(3 + 6);
    graph::meter(Rect::new(x, area.y, w, 1), buf, h / 100.0, &|p| t.humidity(p * 100.0), t.faint(), t.ascii);
    Line::styled(format!("{h:.0}%"), value_style(app, device, t.humidity(h)))
        .alignment(Alignment::Right)
        .render(area, buf);
}

/// One line: `▸ 🌡  Bedside ··········· 17.2°`.
fn compact(app: &App, device: usize, accent: Color, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.devices[device];
    let d = &app.devices[device];
    let selected = device == app.selected;
    let left = Line::from(vec![
        Span::styled(if selected { "▸ " } else { "  " }, Style::new().fg(t.orange())),
        Span::raw(icon(app, device)),
        Span::raw(" "),
        Span::styled(cfg.name.clone(), if selected { t.bold(t.orange()) } else { t.bold(accent) }),
        Span::raw(" "),
    ]);
    let value = match (app.health(device), cfg.kind) {
        (Health::Unpaired, _) => Span::styled("not paired", t.muted()),
        (h @ (Health::Offline | Health::Connecting | Health::Reconnecting), _) if d.current.is_empty() => {
            let (text, c) = health_badge(app, h).unwrap_or((String::new(), t.dim()));
            Span::styled(text, Style::new().fg(c))
        }
        (_, DeviceKind::Plug) => match current(d, Metric::On) {
            Some(v) if v != 0.0 => Span::styled("ON", value_style(app, device, super::glow(app))),
            Some(_) => Span::styled("off", t.muted()),
            None => Span::styled("—", t.muted()),
        },
        _ => match app.shown(device, Metric::Temperature) {
            Some(c) => Span::styled(fmt_temp(app, c), value_style(app, device, t.temperature(c))),
            None => Span::styled("—", t.muted()),
        },
    };
    let right = Line::from(vec![Span::raw(" "), value]);
    // Dotted leaders between name and value, btop style.
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_char(if t.ascii { '.' } else { '·' }).set_style(Style::new().fg(t.faint()));
    }
    left.render(area, buf);
    right.alignment(Alignment::Right).render(area, buf);
}

pub fn transport_name(t: crate::state::Transport) -> &'static str {
    match t {
        crate::state::Transport::WiFi => "wi-fi",
        crate::state::Transport::Thread => "thread",
        crate::state::Transport::Ethernet => "ethernet",
        crate::state::Transport::Unknown => "",
    }
}

pub fn system_mode(m: u8) -> &'static str {
    match m {
        0 => "off",
        1 => "auto",
        3 => "cool",
        4 => "heat",
        5 => "emergency heat",
        6 => "precooling",
        7 => "fan only",
        8 => "dry",
        9 => "sleep",
        _ => "",
    }
}

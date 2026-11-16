//! Devices, drawn the way btop draws its boxes.
//!
//! ```text
//! ┌─┐Bedside┌──────────────────┐12s┌─┐   a box per device, dimmer than its room
//! │Temp:──────────────── 17.2°C ↓│     info rows: label, leader, value
//! │Humidity: ■■■■■■■■■■····· 55%│     or label, meter, value
//! │⣀⣠⣤⣴⣶⣾⣿⣿⣿⣷⣶⣦⣤⣄⣀        18.4°│     braille graph fills the rest
//! └─────────────────────────────┘
//! ```
//!
//! A featured device fills its room instead (like btop's CPU box), and in
//! tight spaces devices become one-line rows with a mini braille graph.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::{btop_box, current, fmt_temp, health_badge, is_leaking, label_row, metric_graph, meter_row, trend};
use crate::config::DeviceKind;
use crate::state::{Metric, unix_minute};
use crate::ui::app::{App, Health};
use crate::ui::graph::{self, Graph};
use crate::ui::layout::{Variant, min_info_rows};
use crate::ui::motion::mix;
use crate::ui::series;
use crate::ui::icons;

pub fn render(app: &App, device: usize, variant: Variant, area: Rect, buf: &mut Buffer) {
    if area.is_empty() {
        return;
    }
    match variant {
        Variant::Row => row_variant(app, device, area, buf),
        Variant::Boxed => boxed(app, device, area, buf),
        Variant::Featured => featured(app, device, area, buf),
    }
}

fn row(area: Rect, n: u16) -> Rect {
    Rect::new(area.x, area.y + n, area.width, 1).intersection(area)
}

/// Values are dimmed when the device isn't live, so stale data looks stale.
fn value_style(app: &App, device: usize, colour: Color) -> Style {
    if app.health(device).is_ok() {
        Style::new().fg(colour).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(app.theme.dim())
    }
}

/// The device's name as a title: reversed in btop's selection colours when
/// selected.
fn title(app: &App, device: usize) -> Span<'static> {
    let t = &app.theme;
    let cfg = &app.config.devices[device];
    let name = match icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), Some(icons::default_for(cfg.kind)), t.ascii) {
        Some(g) => format!("{g} {}", cfg.name),
        None => cfg.name.clone(),
    };
    if device == app.selected {
        Span::styled(name, Style::new().fg(t.title()).bg(t.selected_bg()).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(name, t.bold(t.title()))
    }
}

/// Right-hand status: a health badge, or how long ago it last reported.
fn status(app: &App, device: usize) -> Span<'static> {
    let t = &app.theme;
    match health_badge(app, app.health(device)) {
        Some((text, colour)) => Span::styled(text, Style::new().fg(colour)),
        None => Span::styled(app.devices[device].last_report.map(super::ago).unwrap_or_default(), t.muted()),
    }
}

// --- Info rows ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Info {
    Temp,
    Setpoint,
    State,
    Humidity,
    Battery,
    Power,
    Water,
    Link,
}

/// Info rows for a device, most important first, skipping ones without data.
fn info_rows(app: &App, device: usize) -> Vec<Info> {
    let d = &app.devices[device];
    let has = |m| current(d, m).is_some();
    let rows: &[Info] = match app.config.devices[device].kind {
        DeviceKind::Plug => &[Info::Power, Info::Link],
        DeviceKind::TemperatureSensor => &[Info::Temp, Info::Humidity, Info::Battery],
        DeviceKind::LeakSensor => &[Info::Water, Info::Battery, Info::Link],
        DeviceKind::Thermostat => &[Info::Temp, Info::Setpoint, Info::State, Info::Humidity, Info::Battery],
    };
    rows.iter()
        .copied()
        .filter(|r| match r {
            Info::Humidity => has(Metric::Humidity),
            Info::Battery => has(Metric::Battery) || has(Metric::BatteryLevel),
            Info::Link => d.info.is_some(),
            _ => true,
        })
        .collect()
}

fn draw_info(app: &App, device: usize, info: Info, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    match info {
        Info::Temp => label_row(app, area, buf, "Temp", temperature(app, device)),
        Info::Humidity => {
            let h = app.shown(device, Metric::Humidity).unwrap_or(0.0);
            meter_row(
                app,
                area,
                buf,
                "Humidity",
                h / 100.0,
                &|p| t.graph(super::device_hue(app, device), p),
                Line::styled(format!("{h:.0}%"), value_style(app, device, t.title())),
            );
        }
        Info::Battery => {
            if let Some(b) = current(d, Metric::Battery) {
                let colour = if b < 20.0 { t.bad() } else { t.title() };
                meter_row(app, area, buf, "Battery", b / 100.0, &|p| t.graph(super::device_hue(app, device), p), Line::styled(format!("{b:.0}%"), value_style(app, device, colour)));
            } else {
                // Devices without a percentage report a charge level instead.
                let (text, colour) = match current(d, Metric::BatteryLevel).map_or(0, |l| l as u8) {
                    0 => ("ok", t.title()),
                    1 => ("low", t.warn()),
                    _ => ("replace", t.bad()),
                };
                label_row(app, area, buf, "Battery", Line::styled(text, value_style(app, device, colour)));
            }
        }
        Info::Setpoint => setpoint_row(app, device, area, buf),
        Info::State => label_row(app, area, buf, "State", state(app, device)),
        Info::Power => {
            let (text, colour) = match current(d, Metric::On) {
                Some(v) if v != 0.0 => ("ON", super::glow(app)),
                Some(_) => ("off", t.dim()),
                None => ("—", t.dim()),
            };
            let mut spans = vec![];
            if let Some(since) = since(app, device, Metric::On) {
                spans.push(Span::styled(format!("for {since}  "), t.muted()));
            }
            spans.push(Span::styled(text, value_style(app, device, colour)));
            label_row(app, area, buf, "Power", Line::from(spans));
        }
        Info::Water => {
            let mut spans = vec![];
            if let Some(since) = since(app, device, Metric::Leak) {
                spans.push(Span::styled(format!("for {since}  "), t.muted()));
            }
            spans.push(leak_state(app, device));
            label_row(app, area, buf, "Water", Line::from(spans));
        }
        Info::Link => {
            let link = d.info.as_ref().map_or("", |i| transport_name(i.transport));
            label_row(app, area, buf, "Link", Line::styled(link.to_string(), t.muted()));
        }
    }
}

fn temperature(app: &App, device: usize) -> Line<'static> {
    let t = &app.theme;
    let d = &app.devices[device];
    match app.shown(device, Metric::Temperature) {
        Some(c) => Line::from(vec![
            Span::styled(format!("{} ", trend(d.history.get(&Metric::Temperature), c, unix_minute())), t.muted()),
            Span::styled(fmt_temp_c(app, c), value_style(app, device, t.title())),
        ]),
        None => Line::styled("—", t.muted()),
    }
}

/// "20.5°C" / "68.9°F".
fn fmt_temp_c(app: &App, c: f64) -> String {
    let unit = match app.config.ui.temperature_unit {
        crate::config::TemperatureUnit::Celsius => "C",
        crate::config::TemperatureUnit::Fahrenheit => "F",
    };
    format!("{}{unit}", fmt_temp(app, c))
}

/// Thermostat facts, if discovery has run.
fn thermostat_info(app: &App, device: usize) -> Option<crate::state::ThermostatInfo> {
    app.devices[device].info.as_ref().and_then(|i| i.thermostat)
}

fn mode(app: &App, device: usize) -> Option<u8> {
    current(&app.devices[device], Metric::Mode).map(|m| m as u8)
}

/// `Setpoint: ■■■■■■■●◆■■■■■■ 18.0°`, a gauge between the setpoint limits
/// with the current temperature (●) and setpoint (◆) marked.
fn setpoint_row(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let (lo, hi) = thermostat_info(app, device).and_then(|i| i.heat_limits).unwrap_or((5.0, 30.0));
    let off = mode(app, device) == Some(0);
    let sp = app.shown(device, Metric::HeatSetpoint).filter(|_| !off);
    let value = match sp {
        Some(sp) => Line::styled(fmt_temp(app, sp), value_style(app, device, t.heat())),
        None if off => Line::styled("off", t.muted()),
        None => Line::styled("—", t.muted()),
    };
    let temp = app.shown(device, Metric::Temperature);
    let frac = temp.map_or(0.0, |c| (c - lo) / (hi - lo));
    let meter = meter_row(app, area, buf, "Setpoint", frac, &|p| t.graph(super::device_hue(app, device), p), value);
    if meter.width < 3 {
        return;
    }
    let col = |v: f64| meter.x + (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * f64::from(meter.width - 1)).round() as u16;
    let temp_col = temp.map(col);
    if let Some(c) = temp {
        buf[(col(c), area.y)].set_char(if t.ascii { 'o' } else { '●' }).set_style(t.bold(t.title()));
    }
    if let Some(sp) = sp {
        // Keep both markers visible when they land in the same column.
        let mut x = col(sp);
        if temp_col == Some(x) {
            x = if temp.is_some_and(|c| sp >= c) && x + 1 < meter.right() { x + 1 } else { x.saturating_sub(1).max(meter.x) };
        }
        buf[(x, area.y)].set_char(if t.ascii { '^' } else { '◆' }).set_style(t.bold(t.heat()));
    }
}

/// `heating` / `cooling` / `idle` / `off`, plus mode and demand.
fn state(app: &App, device: usize) -> Line<'static> {
    let t = &app.theme;
    let d = &app.devices[device];
    if mode(app, device) == Some(0) {
        return Line::styled("off", t.muted());
    }
    let running = current(d, Metric::Running).map_or(0, |r| r as u64);
    let demand = current(d, Metric::HeatDemand);
    let (text, colour) = if running & 0b001 != 0 || demand.is_some_and(|p| p > 0.0) {
        ("heating", super::flame(app))
    } else if running & 0b010 != 0 {
        ("cooling", t.cool())
    } else {
        ("idle", t.dim())
    };
    let mut spans = vec![];
    let can_cool = thermostat_info(app, device).is_none_or(|c| c.can_cool);
    if let Some(m) = mode(app, device)
        && !(m == 4 && !can_cool)
    {
        spans.push(Span::styled(format!("{}  ", system_mode(m)), t.muted()));
    }
    if let Some(p) = demand.filter(|p| *p > 0.0) {
        spans.push(Span::styled(format!("{p:.0}%  "), t.muted()));
    }
    spans.push(Span::styled(text, value_style(app, device, colour)));
    Line::from(spans)
}

/// `LEAK` (alarm colours) or `dry`.
fn leak_state(app: &App, device: usize) -> Span<'static> {
    let t = &app.theme;
    match current(&app.devices[device], Metric::Leak) {
        Some(v) if v != 0.0 => Span::styled("LEAK", Style::new().fg(t.bad()).add_modifier(Modifier::BOLD)),
        Some(_) => Span::styled("dry", value_style(app, device, t.title())),
        None => Span::styled("—", t.muted()),
    }
}

/// How long an on/off state (a plug, a leak) has held, from history.
fn since(app: &App, device: usize, metric: Metric) -> Option<String> {
    let d = &app.devices[device];
    let now_v = current(d, metric)?;
    let h = d.history.get(&metric)?;
    let changed = h.buckets().filter(|b| b.last != now_v).last()?;
    let mins = unix_minute() - changed.minute;
    Some(match mins {
        ..1 => "<1m".into(),
        1..60 => format!("{mins}m"),
        _ => format!("{}h{:02}m", mins / 60, mins % 60),
    })
}

/// The graph a device shows: temperature (with setpoint), on/off, or wet/dry.
fn graph_metric(app: &App, device: usize) -> (Metric, Option<Metric>) {
    match app.config.devices[device].kind {
        DeviceKind::Plug => (Metric::On, None),
        DeviceKind::TemperatureSensor => (Metric::Temperature, None),
        DeviceKind::LeakSensor => (Metric::Leak, None),
        DeviceKind::Thermostat => (Metric::Temperature, Some(Metric::HeatSetpoint)),
    }
}

// --- Variants -------------------------------------------------------------

fn boxed(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let selected = device == app.selected;
    // Each device box wears its own hue: a little quieter normally, full
    // when selected, and briefly brighter when the device reports.
    let h = super::device_hue(app, device);
    let full = t.ink(h);
    let base = if selected { full } else { t.shade(h, 0.3) };
    let colour = if is_leaking(&app.devices[device]) { t.bad() } else { mix(base, full, app.flash(device)) };
    ratatui::widgets::Clear.render(area, buf);
    let inner = btop_box(app, area, buf, colour, vec![title(app, device)], vec![status(app, device)]);
    if inner.is_empty() {
        return;
    }
    if app.health(device) == Health::Unpaired {
        label_row(
            app,
            row(inner, 0),
            buf,
            "Not paired",
            Line::from(vec![Span::styled("press ", t.muted()), Span::styled("p", t.bold(t.hi()))]),
        );
        return;
    }
    let rows = info_rows(app, device);
    let min = (min_info_rows(app.config.devices[device].kind) as usize).min(rows.len());
    // Show as many info rows as fit, keeping at least one row of graph.
    let n = rows.len().min((inner.height as usize).saturating_sub(1)).max(min.min(inner.height as usize));
    for (i, info) in rows.iter().take(n).enumerate() {
        draw_info(app, device, *info, row(inner, i as u16), buf);
    }
    let graph = Rect::new(inner.x, inner.y + n as u16, inner.width, inner.height.saturating_sub(n as u16));
    let (m, overlay) = graph_metric(app, device);
    metric_graph(app, device, m, overlay, graph, buf);
}

/// Fills the room like btop's CPU box: a summary line, a gauge or meter, and
/// the graph across everything below.
fn featured(app: &App, device: usize, full: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let kind = app.config.devices[device].kind;
    let d = &app.devices[device];
    // Boxes float over the top-right (like btop's per-core box): text rows
    // stop short of them; the graph runs underneath.
    // Only boxes that reach the text rows at the top matter.
    let overlay_left = app
        .layout
        .tiles
        .iter()
        .filter(|p| p.variant == Variant::Boxed && full.contains(p.rect.as_position()) && p.rect.y <= full.y + 3)
        .map(|p| p.rect.x)
        .min();
    let area = match overlay_left {
        Some(x) => Rect::new(full.x, full.y, x.saturating_sub(full.x + 1), full.height),
        None => full,
    };

    // Name, then the headline values.
    let mut spans = vec![title(app, device), Span::raw("  ")];
    if app.health(device) == Health::Unpaired {
        spans.push(Span::styled("not paired · press ", t.muted()));
        spans.push(Span::styled("p", t.bold(t.hi())));
        Line::from(spans).render(row(area, 0), buf);
        return;
    }
    if kind == DeviceKind::Plug {
        let on = current(d, Metric::On).is_some_and(|v| v != 0.0);
        spans.push(Span::styled(if on { "ON" } else { "off" }, value_style(app, device, if on { super::glow(app) } else { t.dim() })));
    } else if kind == DeviceKind::LeakSensor {
        spans.push(leak_state(app, device));
    } else {
        spans.extend(temperature(app, device).spans);
        if kind == DeviceKind::Thermostat {
            spans.push(Span::raw("   "));
            spans.extend(state(app, device).spans);
        }
    }
    Line::from(spans).render(row(area, 0), buf);
    Line::from(status(app, device)).alignment(Alignment::Right).render(row(area, 0), buf);

    // Second line: the setpoint gauge (thermostats) or humidity meter.
    let mut graph_top = 1;
    match kind {
        DeviceKind::Thermostat => {
            setpoint_row(app, device, row(area, 1), buf);
            graph_top = 2;
        }
        DeviceKind::TemperatureSensor if current(d, Metric::Humidity).is_some() => {
            draw_info(app, device, Info::Humidity, row(area, 1), buf);
            graph_top = 2;
        }
        _ => {}
    }
    if kind == DeviceKind::Thermostat && current(d, Metric::Humidity).is_some() && area.height > 8 {
        draw_info(app, device, Info::Humidity, row(area, 2), buf);
        graph_top = 3;
    }
    let graph = Rect::new(full.x, full.y + graph_top, full.width, full.height.saturating_sub(graph_top));
    let (m, overlay) = graph_metric(app, device);
    metric_graph(app, device, m, overlay, graph, buf);
}

/// One line, like btop's core rows: `Bedside ⣀⣤⣶⣾⣿⣷⣶ 17.2°C`.
fn row_variant(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.devices[device];
    let d = &app.devices[device];
    let name_w = (area.width / 3).clamp(6, 16);
    let mut title = title(app, device);
    let name: String = title.content.chars().take(name_w as usize).collect();
    title.content = name.into();
    Line::from(title).render(Rect::new(area.x, area.y, name_w, 1), buf);

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
        (_, DeviceKind::LeakSensor) => leak_state(app, device),
        _ => match app.shown(device, Metric::Temperature) {
            Some(c) => Span::styled(fmt_temp_c(app, c), value_style(app, device, t.title())),
            None => Span::styled("—", t.muted()),
        },
    };
    let value = Line::from(vec![Span::raw(" "), value]);
    let vw = value.width() as u16;
    value.alignment(Alignment::Right).render(area, buf);

    // Mini braille graph between name and value (dots for missing data,
    // like btop's idle cores).
    let g = Rect::new(area.x + name_w + 1, area.y, area.width.saturating_sub(name_w + 1 + vw + 1), 1);
    if g.width == 0 {
        return;
    }
    for x in g.left()..g.right() {
        buf[(x, g.y)].set_char(if t.ascii { '.' } else { '⣀' }).set_style(Style::new().fg(t.faint()));
    }
    let (m, _) = graph_metric(app, device);
    if let Some(h) = d.history.get(&m) {
        let samples =
            series::sample(h, unix_minute(), super::span(app, h), Graph::samples_for(g.width, t.ascii), super::continuous(m));
        let range = if m.binary() {
            Some((0.0, 1.0))
        } else {
            graph::range(samples.iter().flatten().copied(), 1.5)
        };
        if let Some((lo, hi)) = range {
            let h = super::device_hue(app, device);
            let colour = |frac: f64| t.graph(h, frac);
            Graph { samples: &samples, lo, hi, colour: &colour, overlay: None, ascii: t.ascii }.render(g, buf);
        }
    }
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

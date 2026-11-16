//! Device tiles in their three sizes.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Chart, Dataset, GraphType, Widget};
use tui_big_text::{BigText, PixelSize};

use super::{current, fmt_temp, health_badge, sparkline, timeline, trend};
use crate::config::DeviceKind;
use crate::state::{Metric, unix_minute};
use crate::ui::app::{App, Health};
use crate::ui::icons;
use crate::ui::layout::Variant;
use crate::ui::motion::mix;
use crate::ui::series;

pub fn render(app: &App, device: usize, variant: Variant, accent: Color, area: Rect, buf: &mut Buffer) {
    let selected = device == app.selected;
    match variant {
        Variant::Compact => compact(app, device, accent, selected, area, buf),
        Variant::Normal | Variant::Featured => {
            let inner = frame(app, device, accent, selected, area, buf);
            if app.health(device) == Health::Unpaired {
                let id = app.config.devices[device].id.get_ref();
                Line::styled(format!("mattertui pair {id} <code>"), app.theme.muted()).render(inner, buf);
                return;
            }
            match (variant, app.config.devices[device].kind) {
                (_, DeviceKind::Plug) => plug(app, device, inner, buf),
                (Variant::Featured, _) => featured(app, device, inner, buf),
                (_, DeviceKind::TemperatureSensor) => sensor(app, device, inner, buf),
                (_, DeviceKind::Thermostat) => thermostat(app, device, inner, buf),
            }
        }
    }
}

/// Border, icon and name, health badge. Returns the inner area.
fn frame(app: &App, device: usize, accent: Color, selected: bool, area: Rect, buf: &mut Buffer) -> Rect {
    let t = &app.theme;
    let cfg = &app.config.devices[device];
    let d = &app.devices[device];
    let health = app.health(device);

    let icon_colour = match cfg.kind {
        DeviceKind::Plug if current(d, Metric::On) == Some(1.0) => super::glow(app),
        _ => accent,
    };
    // Selected tiles wear their room's accent; others flash it briefly
    // when the device reports.
    let border = if selected {
        Style::new().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(mix(t.border(), accent, app.flash(device)))
    };
    let name_style = if selected { t.bold(t.fg()) } else { t.text() };
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), t.ascii), Style::new().fg(icon_colour)),
            Span::raw(" "),
            Span::styled(cfg.name.clone(), name_style),
            Span::raw(" "),
        ]));
    if let Some((text, colour)) = health_badge(app, health) {
        block = block.title_bottom(Line::styled(format!(" {text} "), Style::new().fg(colour)).alignment(Alignment::Right));
    } else if let Some(t_last) = d.last_report {
        block = block.title_bottom(
            Line::styled(format!(" {} ", super::ago(t_last)), Style::new().fg(t.faint())).alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    block.render(area, buf);
    inner
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

fn plug(app: &App, device: usize, inner: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    let (text, colour) = match current(d, Metric::On) {
        Some(v) if v != 0.0 => ("ON", super::glow(app)),
        Some(_) => ("OFF", t.dim()),
        None => ("—", t.dim()),
    };
    Line::styled(text, value_style(app, device, colour)).render(row(inner, 0), buf);
    if let Some(info) = &d.info {
        Line::styled(transport(info.transport), t.muted())
            .alignment(Alignment::Right)
            .render(row(inner, 0), buf);
    }
    timeline(app, d.history.get(&Metric::On), row(inner, 1), buf, t.glow());
}

fn sensor(app: &App, device: usize, inner: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    temperature_line(app, device, row(inner, 0), buf);
    battery(app, device, row(inner, 0), buf);
    if let Some(h) = app.shown(device, Metric::Humidity) {
        Line::from(vec![
            Span::styled(format!("{} ", icons::cell("humidity", t.ascii)), Style::new().fg(t.humidity(h))),
            Span::styled(format!("{h:.0}%"), value_style(app, device, t.humidity(h))),
        ])
        .render(row(inner, 1), buf);
    }
    sparkline(app, d, Metric::Temperature, row(inner, 2), buf, |v| t.temperature(v));
}

fn thermostat(app: &App, device: usize, inner: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    temperature_line(app, device, row(inner, 0), buf);
    if let Some(h) = humidity_span(app, device) {
        Line::from(h).alignment(Alignment::Right).render(row(inner, 0), buf);
    }
    setpoint_line(app, device, row(inner, 1), buf);
    running_line(app, device, row(inner, 2), buf);
    battery(app, device, row(inner, 2), buf);
    sparkline(app, d, Metric::Temperature, row(inner, 3), buf, |v| t.temperature(v));
}

fn featured(app: &App, device: usize, inner: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    let kind = app.config.devices[device].kind;

    // Big readout on the left, details on the right.
    let temp = app.shown(device, Metric::Temperature);
    let big = temp.map_or("--.-".into(), |c| fmt_temp(app, c).trim_end_matches('°').to_string());
    let colour = temp.map_or(t.dim(), |c| t.temperature(c));
    let big_w = (big.chars().count() as u16) * 4;
    let big_area = Rect::new(inner.x + 1, inner.y, big_w.min(inner.width), 4).intersection(inner);
    BigText::builder()
        .pixel_size(PixelSize::Quadrant)
        .style(value_style(app, device, colour))
        .lines(vec![big.into()])
        .build()
        .render(big_area, buf);
    let side = Rect::new(big_area.right() + 2, inner.y, inner.width.saturating_sub(big_w + 3), 4);
    if let Some(c) = temp {
        let unit = match app.config.ui.temperature_unit {
            crate::config::TemperatureUnit::Celsius => "°C",
            crate::config::TemperatureUnit::Fahrenheit => "°F",
        };
        Line::from(vec![
            Span::styled(unit, Style::new().fg(colour)),
            Span::styled(format!(" {}", trend(d.history.get(&Metric::Temperature), c, unix_minute())), t.muted()),
        ])
        .render(row(side, 0), buf);
    }
    match kind {
        DeviceKind::Thermostat => {
            if let Some(h) = humidity_span(app, device) {
                Line::from(h).alignment(Alignment::Right).render(row(side, 0), buf);
            }
            setpoint_line(app, device, row(side, 1), buf);
            running_line(app, device, row(side, 2), buf);
            setpoint_gauge(app, device, Rect::new(inner.x + 1, inner.y + 4, inner.width.saturating_sub(2), 1), buf);
        }
        _ => {
            if let Some(h) = humidity_span(app, device) {
                Line::from(h).render(row(side, 1), buf);
            }
        }
    }
    battery_left(app, device, row(side, 3), buf);

    let chart_area = Rect::new(inner.x, inner.y + 5, inner.width, inner.height.saturating_sub(5));
    temperature_chart(app, device, chart_area, buf, false);
}

/// Temperature (+ setpoint for thermostats) line chart.
pub fn temperature_chart(app: &App, device: usize, area: Rect, buf: &mut Buffer, axes: bool) {
    let t = &app.theme;
    let d = &app.devices[device];
    let now = unix_minute();
    let range = app.range_minutes();
    let Some(h) = d.history.get(&Metric::Temperature) else {
        Line::styled("collecting history…", t.muted()).render(area, buf);
        return;
    };
    let temps = series::points(h, now, range);
    let setpoints = d.history.get(&Metric::HeatSetpoint).map(|h| series::points(h, now, range)).unwrap_or_default();
    let (mut lo, mut hi) = temps
        .iter()
        .chain(&setpoints)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, y)| (lo.min(y), hi.max(y)));
    if !lo.is_finite() {
        return;
    }
    // Keep at least a 2° window so a flat line isn't all noise.
    if hi - lo < 2.0 {
        let mid = (hi + lo) / 2.0;
        lo = mid - 1.0;
        hi = mid + 1.0;
    }
    let colour = current(d, Metric::Temperature).map_or(t.dim(), |c| t.temperature(c));
    let marker = if t.ascii { Marker::Dot } else { Marker::Braille };
    let mut sets = vec![
        Dataset::default()
            .marker(marker)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(colour))
            .data(&temps),
    ];
    if !setpoints.is_empty() {
        sets.insert(
            0,
            Dataset::default()
                .marker(marker)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(t.heat()).add_modifier(Modifier::DIM))
                .data(&setpoints),
        );
    }
    let label = |v: f64| Span::styled(fmt_temp(app, v), t.muted());
    let mut x = Axis::default().bounds([-(range as f64), 0.0]);
    let mut y = Axis::default().bounds([lo - 0.2, hi + 0.2]);
    if axes {
        x = x
            .labels([Span::styled(format!("-{}", app.range_label()), t.muted()), Span::styled("now", t.muted())])
            .style(Style::new().fg(t.faint()));
        y = y.labels([label(lo), label(hi)]).style(Style::new().fg(t.faint()));
    }
    Chart::new(sets).x_axis(x).y_axis(y).render(area, buf);
}

fn temperature_line(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    let icon = |c: Color| Span::styled(format!("{} ", icons::cell("thermometer", t.ascii)), Style::new().fg(c));
    match app.shown(device, Metric::Temperature) {
        Some(c) => Line::from(vec![
            icon(t.temperature(c)),
            Span::styled(fmt_temp(app, c), value_style(app, device, t.temperature(c))),
            Span::styled(format!(" {}", trend(d.history.get(&Metric::Temperature), c, unix_minute())), t.muted()),
        ])
        .render(area, buf),
        None => Line::from(vec![icon(t.dim()), Span::styled("—", t.muted())]).render(area, buf),
    }
}

/// Thermostat facts, if discovery has run.
fn thermostat_info(app: &App, device: usize) -> Option<crate::state::ThermostatInfo> {
    app.devices[device].info.as_ref().and_then(|i| i.thermostat)
}

fn mode(app: &App, device: usize) -> Option<u8> {
    current(&app.devices[device], Metric::Mode).map(|m| m as u8)
}

fn setpoint_line(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let mode = mode(app, device);
    let caps = thermostat_info(app, device);
    let (can_heat, can_cool) = caps.map_or((true, true), |c| (c.can_heat, c.can_cool));
    if mode == Some(0) {
        Line::styled("off", t.muted()).render(area, buf);
        return;
    }
    let mut spans = vec![];
    if let Some(sp) = app.shown(device, Metric::HeatSetpoint)
        && can_heat
        && mode != Some(3)
    {
        spans.push(Span::styled(format!("→ {}", fmt_temp(app, sp)), Style::new().fg(t.heat())));
    }
    if let Some(sp) = app.shown(device, Metric::CoolSetpoint)
        && can_cool
        && matches!(mode, Some(1 | 3))
    {
        spans.push(Span::styled(
            format!("  {} {}", icons::cell("cool", t.ascii), fmt_temp(app, sp)),
            Style::new().fg(t.cool()),
        ));
    }
    Line::from(spans).render(area, buf);
    // "heat" on a heating-only device says nothing; other modes do.
    if let Some(m) = mode
        && !(m == 4 && !can_cool)
    {
        Line::styled(system_mode(m), t.muted()).alignment(Alignment::Right).render(area, buf);
    }
}

fn running_line(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    if mode(app, device) == Some(0) {
        return;
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
    Line::styled(text, Style::new().fg(colour)).render(area, buf);
}

/// Where the temperature and setpoint sit between the setpoint limits:
/// `5° ━━━━━━━●━━━━━◆────────── 30°`.
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
    let col = |v: f64| {
        let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        track_x + (f * f64::from(track_w - 1)).round() as u16
    };
    let temp = app.shown(device, Metric::Temperature);
    let filled_to = temp.map_or(track_x, col);
    let (fill, empty, temp_mark, sp_mark) = if t.ascii { ('=', '-', 'o', '^') } else { ('━', '─', '●', '◆') };
    for x in track_x..track_x + track_w {
        let (g, c) = if x <= filled_to && temp.is_some() {
            let v = lo + (hi - lo) * f64::from(x - track_x) / f64::from(track_w - 1);
            (fill, t.temperature(v))
        } else {
            (empty, t.faint())
        };
        buf[(x, area.y)].set_char(g).set_style(Style::new().fg(c));
    }
    let last = track_x + track_w - 1;
    let temp_col = temp.map(col);
    if let Some(c) = temp {
        buf[(col(c), area.y)].set_char(temp_mark).set_style(Style::new().fg(t.temperature(c)).add_modifier(Modifier::BOLD));
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
        buf[(x, area.y)].set_char(sp_mark).set_style(Style::new().fg(t.heat()).add_modifier(Modifier::BOLD));
    }
}

fn battery_span(app: &App, device: usize) -> Option<Span<'static>> {
    let t = &app.theme;
    let d = &app.devices[device];
    let icon = icons::cell("battery", t.ascii);
    if let Some(b) = current(d, Metric::Battery) {
        let colour = if b < 20.0 { t.bad() } else { t.dim() };
        return Some(Span::styled(format!("{icon} {b:.0}%"), Style::new().fg(colour)));
    }
    // Devices without a percentage report a charge level instead.
    let (text, colour) = match current(d, Metric::BatteryLevel)? as u8 {
        0 => ("ok", t.dim()),
        1 => ("low", t.warn()),
        _ => ("replace", t.bad()),
    };
    Some(Span::styled(format!("{icon} {text}"), Style::new().fg(colour)))
}

fn humidity_span(app: &App, device: usize) -> Option<Span<'static>> {
    let t = &app.theme;
    let h = app.shown(device, Metric::Humidity)?;
    Some(Span::styled(format!("{} {h:.0}%", icons::cell("humidity", t.ascii)), Style::new().fg(t.humidity(h))))
}

fn battery(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    if let Some(s) = battery_span(app, device) {
        Line::from(s).alignment(Alignment::Right).render(area, buf);
    }
}

fn battery_left(app: &App, device: usize, area: Rect, buf: &mut Buffer) {
    if let Some(s) = battery_span(app, device) {
        Line::from(s).render(area, buf);
    }
}

fn compact(app: &App, device: usize, accent: Color, selected: bool, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.devices[device];
    let d = &app.devices[device];
    let on = current(d, Metric::On) == Some(1.0);
    let marker = if selected { "▸" } else { " " };
    Line::from(vec![
        Span::styled(marker, Style::new().fg(accent)),
        Span::styled(
            icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), t.ascii),
            Style::new().fg(if on { t.glow() } else { accent }),
        ),
        Span::raw(" "),
        Span::styled(cfg.name.clone(), if selected { t.bold(t.fg()) } else { t.text() }),
    ])
    .render(area, buf);

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
    Line::from(vec![value, Span::raw(" ")]).alignment(Alignment::Right).render(area, buf);
}

fn transport(t: crate::state::Transport) -> &'static str {
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

pub fn transport_name(t: crate::state::Transport) -> &'static str {
    transport(t)
}

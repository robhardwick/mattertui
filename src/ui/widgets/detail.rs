//! Full-screen view of one device: facts, health and a chart per metric.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use super::tile::{system_mode, transport_name};
use super::{ago, current, fmt_temp, health_badge, room, timeline};
use crate::state::{Metric, Status, unix_minute};
use crate::ui::app::App;
use crate::ui::icons;
use crate::ui::series;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let i = app.selected;
    let cfg = &app.config.devices[i];
    let d = &app.devices[i];
    let room_idx = app.rooms.iter().position(|r| r.contains(&i)).unwrap_or(0);
    let accent = room::accent(app, room_idx);

    let bracket = |c: &'static str| Span::styled(c, Style::new().fg(accent));
    let block = Block::bordered()
        .border_type(if t.ascii { BorderType::Plain } else { BorderType::Rounded })
        .border_style(Style::new().fg(accent))
        .title(Line::from(vec![
            bracket("┤"),
            Span::raw(format!(
                " {} ",
                icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), t.ascii)
            )),
            Span::styled(cfg.name.clone(), t.bold(t.title())),
            Span::styled(format!(" · {} ", app.config.rooms[room_idx].name), Style::new().fg(accent)),
            bracket("├"),
        ]))
        .title_bottom(
            Line::from(vec![
                bracket("┤"),
                Span::styled(" ←→", t.bold(t.orange())),
                Span::styled(" device · ", t.muted()),
                Span::styled("[ ]", t.bold(t.orange())),
                Span::styled(" range · ", t.muted()),
                Span::styled("Esc", t.bold(t.orange())),
                Span::styled(" back ", t.muted()),
                bracket("├"),
            ])
            .alignment(Alignment::Right),
        );
    let inner = block.inner(area);
    block.render(area, buf);
    let inner = Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);

    // Facts.
    let mut facts: Vec<Span> = Vec::new();
    if let Some(info) = &d.info {
        facts.push(Span::styled(
            format!("{} {}", info.vendor.as_deref().unwrap_or("?"), info.product.as_deref().unwrap_or("?")),
            t.text(),
        ));
        let mut rest = vec![
            info.firmware.as_ref().map(|f| format!("fw {f}")),
            Some(transport_name(info.transport).to_string()).filter(|s| !s.is_empty()),
            Some(format!("node 0x{:X}", info.node_id)),
            Some(format!("endpoint {}", info.endpoint)),
        ];
        if let Some(th) = info.thermostat {
            rest.push(Some(
                match (th.can_heat, th.can_cool) {
                    (true, false) => "heating only",
                    (false, true) => "cooling only",
                    _ => "heating and cooling",
                }
                .to_string(),
            ));
            let whole = |v: f64| fmt_temp(app, v).replace(".0°", "°");
            rest.push(th.heat_limits.map(|(lo, hi)| format!("setpoint {}–{}", whole(lo), whole(hi))));
        }
        for s in rest.into_iter().flatten() {
            facts.push(Span::styled(format!("  ·  {s}"), t.muted()));
        }
    } else {
        facts.push(Span::styled(format!("id {}", cfg.id.get_ref()), t.muted()));
    }
    // Facts can be long (thermostats): wrap onto a second line.
    ratatui::widgets::Paragraph::new(Line::from(facts))
        .wrap(ratatui::widgets::Wrap { trim: true })
        .render(Rect::new(inner.x, inner.y, inner.width, 2).intersection(inner), buf);

    // Health.
    let health = app.health(i);
    let mut status = match health_badge(app, health) {
        None => vec![Span::styled("● live", Style::new().fg(t.good()))],
        Some((text, c)) => {
            let text = if health == crate::ui::app::Health::Reconnecting { "reconnecting" } else { &text };
            vec![Span::styled(format!("● {text}"), Style::new().fg(c))]
        }
    };
    match &d.status {
        Status::Live { max_interval_s } => {
            status.push(Span::styled(format!("  ·  reports at least every {max_interval_s}s"), t.muted()))
        }
        Status::Offline { error } | Status::Reconnecting { cause: error } => {
            status.push(Span::styled(format!("  ·  {error}"), t.muted()))
        }
        Status::Unpaired => status.push(Span::styled(
            format!("  ·  run `mattertui pair {} <code>`", cfg.id.get_ref()),
            t.muted(),
        )),
        Status::Connecting => {}
    }
    if let Some(last) = d.last_report {
        status.push(Span::styled(format!("  ·  last report {} ago", ago(last)), t.muted()));
    }
    Line::from(status).render(row(inner, 2), buf);

    // Range tabs.
    let mut tabs = vec![Span::styled("range ", t.muted())];
    for (label, active) in app.ranges() {
        tabs.push(if active {
            Span::styled(format!(" {label} "), Style::new().fg(Color::Black).bg(t.orange()).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(format!(" {label} "), t.muted())
        });
    }
    Line::from(tabs).render(row(inner, 4), buf);

    // One section per metric with history.
    let body = Rect::new(inner.x, inner.y + 6, inner.width, inner.height.saturating_sub(6));
    let charts: Vec<Metric> = [Metric::Temperature, Metric::Humidity, Metric::HeatDemand, Metric::Battery]
        .into_iter()
        .filter(|m| d.history.contains_key(m))
        .collect();
    let timelines: Vec<Metric> = [Metric::On, Metric::Running]
        .into_iter()
        .filter(|m| d.history.contains_key(m))
        .collect();
    if charts.is_empty() && timelines.is_empty() {
        Line::styled("no history yet: values appear here as the device reports them", t.muted())
            .render(row(body, 0), buf);
        return;
    }
    let timeline_h = 3 * timelines.len() as u16;
    let chart_h = if charts.is_empty() {
        0
    } else {
        body.height.saturating_sub(timeline_h) / charts.len() as u16
    };
    let mut y = body.y;
    for m in charts {
        let area = Rect::new(body.x, y, body.width, chart_h);
        section_title(app, i, m, row(area, 0), buf);
        let chart = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(2));
        graph_with_axes(app, i, m, chart, buf);
        y += chart_h;
    }
    for m in timelines {
        let area = Rect::new(body.x, y, body.width, 3);
        section_title(app, i, m, row(area, 0), buf);
        let colour = if m == Metric::Running { t.heat() } else { t.glow() };
        timeline(app, d.history.get(&m), row(area, 1), buf, colour);
        y += 3;
    }
}

fn row(area: Rect, n: u16) -> Rect {
    Rect::new(area.x, area.y + n, area.width, 1).intersection(area)
}

fn section_title(app: &App, device: usize, m: Metric, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let d = &app.devices[device];
    let name = match m {
        Metric::Temperature => "Temperature",
        Metric::Humidity => "Humidity",
        Metric::Battery => "Battery",
        Metric::HeatDemand => "Heating demand",
        Metric::On => "Power",
        Metric::Running => "Running",
        _ => "",
    };
    let mut spans = vec![Span::styled(name, t.bold(t.fg()))];
    let fmt = |v: f64| match m {
        Metric::Temperature => fmt_temp(app, v),
        _ => format!("{v:.0}%"),
    };
    match m {
        Metric::On | Metric::Running => {
            let label = match (m, current(d, m)) {
                (Metric::On, Some(v)) if v != 0.0 => "on",
                (Metric::On, Some(_)) => "off",
                (Metric::Running, Some(v)) if v as u64 & 1 != 0 => "heating",
                (Metric::Running, Some(v)) if v as u64 & 2 != 0 => "cooling",
                (Metric::Running, Some(_)) => "idle",
                _ => "",
            };
            spans.push(Span::styled(format!("  {label}"), t.muted()));
            if m == Metric::Running && d.info.as_ref().and_then(|i| i.thermostat).is_some_and(|th| th.running_estimated) {
                spans.push(Span::styled(" · estimated from temperature vs setpoint", t.muted()));
            }
            if m == Metric::Running
                && let Some(mode) = current(d, Metric::Mode)
            {
                spans.push(Span::styled(format!(" · mode {}", system_mode(mode as u8)), t.muted()));
            }
        }
        _ => {
            if let Some(h) = d.history.get(&m)
                && let Some(s) = series::stats(h, unix_minute(), app.range_minutes())
            {
                spans.push(Span::styled(
                    format!("  min {}  avg {}  max {}", fmt(s.min), fmt(s.avg), fmt(s.max)),
                    t.muted(),
                ));
            }
        }
    }
    Line::from(spans).render(area, buf);
}

/// A gradient graph with a scale gutter on the left and a time axis below.
fn graph_with_axes(app: &App, device: usize, m: Metric, area: Rect, buf: &mut Buffer) {
    const GUTTER: u16 = 8;
    let t = &app.theme;
    if area.height < 2 || area.width <= GUTTER {
        return;
    }
    let plot = Rect::new(area.x + GUTTER, area.y, area.width - GUTTER, area.height - 1);
    let overlay = (m == Metric::Temperature && app.devices[device].history.contains_key(&Metric::HeatSetpoint))
        .then_some(Metric::HeatSetpoint);
    super::metric_graph_bare(app, device, m, overlay, plot, buf);

    // Scale: the graph's range, top and bottom, plus a midpoint if tall.
    if let Some((lo, hi)) = super::metric_range(app, device, m, overlay, plot.width) {
        let fmt = |v: f64| match m {
            Metric::Temperature => fmt_temp(app, v),
            _ => format!("{v:.0}%"),
        };
        let label = |y: u16, v: f64, buf: &mut Buffer| {
            Line::styled(format!("{} ┤", fmt(v)), Style::new().fg(t.dim()))
                .alignment(Alignment::Right)
                .render(Rect::new(area.x, y, GUTTER, 1), buf);
        };
        label(plot.y, hi, buf);
        label(plot.bottom() - 1, lo, buf);
        if plot.height >= 6 {
            label(plot.y + plot.height / 2, (hi + lo) / 2.0, buf);
        }
    }
    // Time axis.
    let axis = Rect::new(plot.x, plot.bottom(), plot.width, 1);
    for x in axis.left()..axis.right() {
        buf[(x, axis.y)].set_char(if t.ascii { '-' } else { '─' }).set_style(Style::new().fg(t.faint()));
    }
    Line::styled(format!("-{} ", app.range_label()), t.muted()).render(axis, buf);
    Line::styled(" now", t.muted()).alignment(Alignment::Right).render(axis, buf);
}

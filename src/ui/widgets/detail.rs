//! Full-screen view of one device: facts, health and a chart per metric.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Chart, Dataset, GraphType, Widget};

use super::tile::{system_mode, temperature_chart, transport_name};
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

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(accent))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), t.ascii),
                Style::new().fg(accent),
            ),
            Span::raw(" "),
            Span::styled(cfg.name.clone(), t.bold(t.fg())),
            Span::styled(format!(" · {} ", app.config.rooms[room_idx].name), Style::new().fg(accent)),
        ]))
        .title_bottom(
            Line::styled(" ←→ device · [ ] range · Esc back ", Style::new().fg(t.faint())).alignment(Alignment::Right),
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
        if let Some(icd) = info.icd {
            rest.push(Some(format!(
                "battery ICD ({}{})",
                if icd.long_idle_time { "LIT" } else { "SIT" },
                icd.idle_mode_s.map_or(String::new(), |s| format!(", sleeps {s}s")),
            )));
        }
        for s in rest.into_iter().flatten() {
            facts.push(Span::styled(format!("  ·  {s}"), t.muted()));
        }
    } else {
        facts.push(Span::styled(format!("id {}", cfg.id.get_ref()), t.muted()));
    }
    Line::from(facts).render(row(inner, 0), buf);

    // Health.
    let health = app.health(i);
    let mut status = match health_badge(app, health) {
        None => vec![Span::styled("● live", Style::new().fg(t.good()))],
        Some((text, c)) => {
            let text = if health == crate::ui::app::Health::Reconnecting { "reconnecting" } else { text };
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
    Line::from(status).render(row(inner, 1), buf);

    // Range tabs.
    let mut tabs = vec![Span::styled("range ", t.muted())];
    for (label, active) in app.ranges() {
        tabs.push(if active {
            Span::styled(format!(" {label} "), Style::new().fg(Color::Black).bg(accent).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(format!(" {label} "), t.muted())
        });
    }
    Line::from(tabs).render(row(inner, 3), buf);

    // One section per metric with history.
    let body = Rect::new(inner.x, inner.y + 5, inner.width, inner.height.saturating_sub(5));
    let charts: Vec<Metric> = [Metric::Temperature, Metric::Humidity, Metric::Battery]
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
        match m {
            Metric::Temperature => temperature_chart(app, i, chart, buf, true),
            Metric::Humidity => line_chart(app, i, m, chart, buf, |v| t.humidity(v), |v| format!("{v:.0}%")),
            _ => line_chart(app, i, m, chart, buf, |_| t.dim(), |v| format!("{v:.0}%")),
        }
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

fn line_chart(
    app: &App,
    device: usize,
    m: Metric,
    area: Rect,
    buf: &mut Buffer,
    colour: impl Fn(f64) -> Color,
    label: impl Fn(f64) -> String,
) {
    let t = &app.theme;
    let Some(h) = app.devices[device].history.get(&m) else { return };
    let range = app.range_minutes();
    let pts = series::points(h, unix_minute(), range);
    let (mut lo, mut hi) = pts
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &(_, y)| (lo.min(y), hi.max(y)));
    if !lo.is_finite() {
        return;
    }
    if hi - lo < 4.0 {
        let mid = (hi + lo) / 2.0;
        lo = mid - 2.0;
        hi = mid + 2.0;
    }
    let c = current(&app.devices[device], m).map_or(t.dim(), &colour);
    let set = Dataset::default()
        .marker(if t.ascii { Marker::Dot } else { Marker::Braille })
        .graph_type(GraphType::Line)
        .style(Style::new().fg(c))
        .data(&pts);
    Chart::new(vec![set])
        .x_axis(
            Axis::default()
                .bounds([-(range as f64), 0.0])
                .labels([Span::styled(format!("-{}", app.range_label()), t.muted()), Span::styled("now", t.muted())])
                .style(Style::new().fg(t.faint())),
        )
        .y_axis(
            Axis::default()
                .bounds([lo, hi])
                .labels([Span::styled(label(lo), t.muted()), Span::styled(label(hi), t.muted())])
                .style(Style::new().fg(t.faint())),
        )
        .render(area, buf);
}

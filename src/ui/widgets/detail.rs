//! Full-screen view of one device, btop style: facts as label rows, then a
//! box per metric with a braille graph, scale and time axis.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::tile::{system_mode, transport_name};
use super::{ago, btop_box, current, fmt_temp, health_badge, label_row, room};
use crate::state::{Metric, Status, unix_minute};
use crate::ui::app::{App, Health};
use crate::ui::series;

pub fn render(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let i = app.selected;
    let cfg = &app.config.devices[i];
    let d = &app.devices[i];
    let room_idx = app.rooms.iter().position(|r| r.contains(&i)).unwrap_or(0);
    let colour = room::accent(app, room_idx);

    let outer = btop_box(
        app,
        area,
        buf,
        colour,
        vec![
            Span::styled(cfg.name.clone(), t.bold(t.title())),
            Span::styled(format!(" {}", app.config.rooms[room_idx].name), t.muted()),
        ],
        vec![
            Span::styled("←→", t.bold(t.hi())),
            Span::styled(" device ", t.muted()),
            Span::styled("[ ]", t.bold(t.hi())),
            Span::styled(" range ", t.muted()),
            Span::styled("Esc", t.bold(t.hi())),
            Span::styled(" back", t.muted()),
        ],
    );
    let inner = Rect::new(outer.x + 1, outer.y, outer.width.saturating_sub(2), outer.height);
    let mut y = 0;
    let mut row = |buf: &mut Buffer, label: &str, value: Line<'static>| {
        if y < inner.height {
            label_row(app, Rect::new(inner.x, inner.y + y, inner.width, 1), buf, label, value);
            y += 1;
        }
    };

    // Facts.
    if let Some(info) = &d.info {
        row(
            buf,
            "Device",
            Line::from(vec![
                Span::styled(
                    format!("{} {}", info.vendor.as_deref().unwrap_or("?"), info.product.as_deref().unwrap_or("?")),
                    t.bold(t.title()),
                ),
                Span::styled(info.firmware.as_ref().map(|f| format!("  fw {f}")).unwrap_or_default(), t.muted()),
            ]),
        );
        row(
            buf,
            "Network",
            Line::styled(
                format!("{}  node 0x{:X}  endpoint {}", transport_name(info.transport), info.node_id, info.endpoint),
                t.muted(),
            ),
        );
        if let Some(th) = info.thermostat {
            let whole = |v: f64| fmt_temp(app, v).replace(".0°", "°");
            let caps = match (th.can_heat, th.can_cool) {
                (true, false) => "heating only",
                (false, true) => "cooling only",
                _ => "heating and cooling",
            };
            let limits = th.heat_limits.map(|(lo, hi)| format!("  setpoint {}–{}", whole(lo), whole(hi))).unwrap_or_default();
            row(buf, "Control", Line::styled(format!("{caps}{limits}"), t.muted()));
        }
    } else {
        row(buf, "Device", Line::styled(cfg.id.get_ref().clone(), t.muted()));
    }

    // Health.
    let health = app.health(i);
    let mut status = vec![];
    match &d.status {
        Status::Live { max_interval_s } => status.push(Span::styled(format!("reports every ≤{max_interval_s}s  "), t.muted())),
        Status::Offline { error } | Status::Reconnecting { cause: error } => {
            status.push(Span::styled(format!("{error}  "), t.muted()))
        }
        Status::Unpaired => status.push(Span::styled(format!("mattertui pair {} <code>  ", cfg.id.get_ref()), t.muted())),
        Status::Connecting => {}
    }
    if let Some(last) = d.last_report {
        status.push(Span::styled(format!("last {} ago  ", ago(last)), t.muted()));
    }
    status.push(match health_badge(app, health) {
        None => Span::styled("live", t.bold(t.good())),
        Some((text, c)) => {
            let text = if health == Health::Reconnecting { "reconnecting".to_string() } else { text };
            Span::styled(text, t.bold(c))
        }
    });
    row(buf, "Status", Line::from(status));

    // Range tabs.
    let mut tabs = vec![];
    for (label, active) in app.ranges() {
        tabs.push(if active {
            Span::styled(format!(" {label} "), Style::new().fg(t.title()).bg(t.selected_bg()).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(format!(" {label} "), t.muted())
        });
    }
    row(buf, "Range", Line::from(tabs));

    // A box per metric with history.
    let body = Rect::new(outer.x, outer.y + y + 1, outer.width, outer.height.saturating_sub(y + 1));
    let charts: Vec<Metric> = [Metric::Temperature, Metric::Humidity, Metric::HeatDemand, Metric::Battery]
        .into_iter()
        .filter(|m| d.history.contains_key(m))
        .collect();
    let switches: Vec<Metric> = [Metric::On, Metric::Running]
        .into_iter()
        .filter(|m| d.history.contains_key(m))
        .collect();
    if charts.is_empty() && switches.is_empty() {
        Line::styled(" no history yet: graphs appear as the device reports", t.muted())
            .render(Rect::new(body.x, body.y, body.width, 1), buf);
        return;
    }
    // On/off boxes are short; charts share what's left.
    const SWITCH_H: u16 = 5;
    let switch_total = SWITCH_H * switches.len() as u16;
    let chart_h = if charts.is_empty() { 0 } else { body.height.saturating_sub(switch_total) / charts.len() as u16 };
    let mut y = body.y;
    for m in charts {
        let r = Rect::new(body.x, y, body.width, chart_h);
        metric_box(app, i, m, r, buf);
        y += chart_h;
    }
    for m in switches {
        let r = Rect::new(body.x, y, body.width, SWITCH_H.min(body.bottom().saturating_sub(y)));
        metric_box(app, i, m, r, buf);
        y += SWITCH_H;
    }
}

fn metric_name(m: Metric) -> &'static str {
    match m {
        Metric::Temperature => "temperature",
        Metric::Humidity => "humidity",
        Metric::Battery => "battery",
        Metric::HeatDemand => "heating demand",
        Metric::On => "power",
        Metric::Running => "heating",
        _ => "",
    }
}

/// A metric's box: title, stats on the right, graph with scale and axis.
fn metric_box(app: &App, device: usize, m: Metric, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    if area.height < 3 {
        return;
    }
    let d = &app.devices[device];
    let mut right = vec![];
    match m {
        Metric::On | Metric::Running => {
            let now = current(d, m).unwrap_or(0.0) as u64;
            let (text, colour) = match m {
                Metric::On if now != 0 => ("on", super::glow(app)),
                Metric::On => ("off", t.dim()),
                _ if now & 1 != 0 => ("heating", super::flame(app)),
                _ => ("idle", t.dim()),
            };
            if m == Metric::Running {
                if d.info.as_ref().and_then(|i| i.thermostat).is_some_and(|th| th.running_estimated) {
                    right.push(Span::styled("estimated  ", t.muted()));
                }
                if let Some(mode) = current(d, Metric::Mode) {
                    right.push(Span::styled(format!("mode {}  ", system_mode(mode as u8)), t.muted()));
                }
            }
            right.push(Span::styled(text, t.bold(colour)));
        }
        _ => {
            let fmt = |v: f64| match m {
                Metric::Temperature => fmt_temp(app, v),
                _ => format!("{v:.0}%"),
            };
            if let Some(h) = d.history.get(&m)
                && let Some(s) = series::stats(h, unix_minute(), app.range_minutes())
            {
                for (label, v) in [("min ", s.min), ("  avg ", s.avg), ("  max ", s.max)] {
                    right.push(Span::styled(label, t.muted()));
                    right.push(Span::styled(fmt(v), t.bold(t.title())));
                }
            }
        }
    }
    let colour = t.ink(super::device_hue(app, device));
    let inner = btop_box(app, area, buf, colour, vec![Span::styled(metric_name(m), t.bold(t.title()))], right);
    graph_with_axes(app, device, m, inner, buf);
}

/// A braille graph with a scale gutter on the left and a time axis below.
fn graph_with_axes(app: &App, device: usize, m: Metric, area: Rect, buf: &mut Buffer) {
    const GUTTER: u16 = 8;
    let t = &app.theme;
    if area.height < 2 || area.width <= GUTTER {
        return;
    }
    let plot = Rect::new(area.x + GUTTER, area.y, area.width - GUTTER - 1, area.height - 1);
    let overlay = (m == Metric::Temperature && app.devices[device].history.contains_key(&Metric::HeatSetpoint))
        .then_some(Metric::HeatSetpoint);
    super::metric_graph_bare(app, device, m, overlay, plot, buf);

    if let Some((lo, hi)) = super::metric_range(app, device, m, overlay, plot.width) {
        let fmt = |v: f64| match m {
            Metric::Temperature => fmt_temp(app, v),
            Metric::On | Metric::Running => (if v >= 0.5 { "on" } else { "off" }).to_string(),
            _ => format!("{v:.0}%"),
        };
        let label = |y: u16, v: f64, buf: &mut Buffer| {
            Line::styled(format!("{} ", fmt(v)), t.muted())
                .alignment(Alignment::Right)
                .render(Rect::new(area.x, y, GUTTER, 1), buf);
        };
        label(plot.y, hi, buf);
        if plot.height > 1 {
            label(plot.bottom() - 1, lo, buf);
        }
        if plot.height >= 6 {
            label(plot.y + plot.height / 2, (hi + lo) / 2.0, buf);
        }
    }
    // Time axis.
    let axis = Rect::new(plot.x, plot.bottom(), plot.width, 1);
    for x in axis.left()..axis.right() {
        buf[(x, axis.y)].set_char('─').set_style(Style::new().fg(t.faint()));
    }
    // The span actually drawn (shorter than the range while history builds).
    let shown = app.devices[device].history.get(&m).map_or(app.range_minutes(), |h| super::span(app, h));
    Line::styled(format!("-{} ", super::fmt_span(shown)), t.muted()).render(axis, buf);
    Line::styled(" now", t.muted()).alignment(Alignment::Right).render(axis, buf);
}

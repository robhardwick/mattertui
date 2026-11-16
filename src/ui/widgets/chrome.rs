//! Top and bottom rules, the log box and the help box.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};
use tracing::Level;

use super::{btop_box, current};
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;
use crate::ui::icons;

/// Height of the top panel: a border and one row of content.
pub const TOP_HEIGHT: u16 = 3;

/// The top panel, btop style: the name and clock in the border, and a row
/// of status:
/// `│● 5/6 live   ☼ 2 on   ≋ 1 heating   up 2h13m   history 24h   ?help q│`
pub fn top_panel(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let total = app.devices.len();
    let live = (0..total).filter(|&i| app.health(i).is_ok()).count();
    let on = |i: usize, m: Metric| current(&app.devices[i], m).is_some_and(|v| v != 0.0);
    let lights_on = (0..total).filter(|&i| app.config.devices[i].kind == DeviceKind::Plug && on(i, Metric::On)).count();
    let heating = app.devices.iter().filter(|d| super::is_heating(d)).count();

    // The top panel wears the anchor hue, orange.
    let edge = t.ink(crate::ui::theme::hue::ORANGE);
    let inner = btop_box(
        app,
        area,
        buf,
        edge,
        vec![Span::styled("mattertui", t.bold(t.title()))],
        vec![Span::styled(app.range_label(), t.bold(t.title())), Span::styled(" range", t.muted())],
    );
    // The clock sits in the middle of the top border, as in btop.
    let clock = Line::from(vec![
        Span::styled("┐", Style::new().fg(edge)),
        Span::styled(jiff::ZonedDateTime::now().strftime("%H:%M:%S").to_string(), t.bold(t.title())),
        Span::styled("┌", Style::new().fg(edge)),
    ]);
    let cw = clock.width() as u16;
    if area.width > cw + 50 {
        clock.render(Rect::new(area.x + (area.width - cw) / 2, area.y, cw, 1), buf);
    }
    if inner.is_empty() {
        return;
    }

    let gap = || Span::raw("   ");
    let mut spans = vec![
        Span::raw(" "),
        Span::styled("● ", Style::new().fg(if live == total { t.good() } else { t.warn() })),
        Span::styled(format!("{live}/{total}"), t.bold(t.title())),
        Span::styled(" live", t.muted()),
        gap(),
        Span::styled(format!("{} ", icons::get("lamp", t.ascii)), if lights_on > 0 { Style::new().fg(super::glow(app)) } else { t.muted() }),
        Span::styled(lights_on.to_string(), t.bold(t.title())),
        Span::styled(" on", t.muted()),
    ];
    if heating > 0 {
        spans.extend([
            gap(),
            Span::styled(format!("{} ", icons::get("heating", t.ascii)), Style::new().fg(super::flame(app))),
            Span::styled(heating.to_string(), t.bold(t.title())),
            Span::styled(" heating", t.muted()),
        ]);
    }
    spans.extend([
        gap(),
        Span::styled("up ", t.muted()),
        Span::styled(uptime(app.epoch.elapsed().as_secs()), t.bold(t.title())),
        gap(),
        Span::styled("history ", t.muted()),
        Span::styled(format!("{}h", app.config.ui.history_hours), t.bold(t.title())),
    ]);
    let unseen = app.unseen_warnings();
    if unseen > 0 && !app.log_open {
        spans.extend([gap(), Span::styled(format!("{unseen} warnings"), t.bold(t.warn()))]);
    }
    let left = Line::from(spans);
    let hints = Line::from(vec![
        Span::styled("?", t.bold(t.hi())),
        Span::styled("help  ", t.muted()),
        Span::styled("`", t.bold(t.hi())),
        Span::styled("log  ", t.muted()),
        Span::styled("q", t.bold(t.hi())),
        Span::styled("uit ", t.muted()),
    ]);
    let lw = left.width() as u16;
    left.render(inner, buf);
    if lw + hints.width() as u16 + 2 <= inner.width {
        hints.alignment(Alignment::Right).render(inner, buf);
    }
}

/// "42m", "3h05m", "2d 4h".
fn uptime(secs: u64) -> String {
    let m = secs / 60;
    match m {
        0..60 => format!("{m}m"),
        60..1440 => format!("{}h{:02}m", m / 60, m % 60),
        _ => format!("{}d {}h", m / 1440, (m % 1440) / 60),
    }
}

/// The log, in its own box.
pub fn log_panel(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let lines = app.logs.snapshot();
    // The log wears sky; brighter while focused for scrolling.
    let sky = crate::ui::theme::hue::SKY;
    let colour = if app.log_focus { t.ink(sky) } else { t.shade(sky, 0.3) };
    let left = vec![Span::styled("log", t.bold(t.title()))];
    let right = if app.log_scroll == 0 {
        vec![]
    } else {
        vec![Span::styled(format!("paused ↑{}", app.log_scroll), t.bold(t.warn())), Span::styled(" End", t.bold(t.hi())), Span::styled(" follow", t.muted())]
    };
    let inner = btop_box(app, area, buf, colour, left, right);

    let height = inner.height as usize;
    let end = lines.len().saturating_sub(app.log_scroll);
    let start = end.saturating_sub(height);
    for (row, l) in lines[start..end].iter().enumerate() {
        let level_colour = match l.level {
            Level::ERROR => t.bad(),
            Level::WARN => t.warn(),
            Level::INFO => t.good(),
            _ => t.dim(),
        };
        let mut spans = vec![
            Span::styled(format!("{} ", l.time), t.muted()),
            Span::styled(format!("{:<5} ", l.level.as_str()), t.bold(level_colour)),
        ];
        if let Some(dev) = &l.device {
            let colour = device_accent(app, dev).unwrap_or(t.dim());
            let name = app.config.device(dev).map_or(dev.as_str(), |d| d.name.as_str());
            spans.push(Span::styled(format!("{name} "), t.bold(colour)));
        } else if l.target != "mattertui" && l.target != "event" {
            spans.push(Span::styled(format!("{} ", l.target), t.muted()));
        }
        spans.push(Span::styled(l.message.clone(), t.text()));
        if !l.fields.is_empty() {
            spans.push(Span::styled(format!(" {}", l.fields), t.muted()));
        }
        Line::from(spans).render(Rect::new(inner.x, inner.y + row as u16, inner.width, 1), buf);
    }
}

fn device_accent(app: &App, id: &str) -> Option<Color> {
    let i = app.config.devices.iter().position(|d| d.id.get_ref() == id)?;
    let room = app.rooms.iter().position(|r| r.contains(&i))?;
    Some(super::room::accent(app, room))
}

const HELP: &[(&str, &str)] = &[
    ("←↓↑→ hjkl", "move between devices"),
    ("1-9", "jump to a room"),
    ("Enter", "device detail (Esc back)"),
    ("[ ]", "graph range"),
    ("m", "toggle motion"),
    ("p", "pair the selected device"),
    ("U", "unpair the selected device"),
    ("`", "toggle log"),
    ("Tab", "scroll log (↑↓ PgUp PgDn End)"),
    ("?", "this help"),
    ("q", "quit"),
];

pub fn help(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let w = 50.min(area.width);
    let h = (HELP.len() as u16 + 4).min(area.height);
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    Clear.render(r, buf);
    let inner = btop_box(app, r, buf, t.hi(), vec![Span::styled("help", t.bold(t.title()))], vec![]);
    for (i, (k, v)) in HELP.iter().enumerate() {
        Line::from(vec![
            Span::styled(format!(" {k:<12}"), t.bold(t.hi())),
            Span::styled(*v, t.text()),
        ])
        .render(Rect::new(inner.x, inner.y + 1 + i as u16, inner.width, 1), buf);
    }
}

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

/// Brackets a title into a rule, btop style: `─┐title┌─`.
fn bracketed(edge: Color, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut v = vec![Span::styled("┐", Style::new().fg(edge))];
    v.extend(spans);
    v.push(Span::styled("┌", Style::new().fg(edge)));
    v
}

fn rule(area: Rect, buf: &mut Buffer, colour: Color) {
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_char('─').set_style(Style::new().fg(colour));
    }
}

/// `──┐mattertui┌─┐5/6 live┌─┐lights 2┌────┐00:12┌────┐1h┌─┐?help┌──`
pub fn top_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let edge = t.frame();
    rule(area, buf, edge);
    let total = app.devices.len();
    let live = (0..total).filter(|&i| app.health(i).is_ok()).count();
    let lights_on = (0..total)
        .filter(|&i| app.config.devices[i].kind == DeviceKind::Plug && current(&app.devices[i], Metric::On) == Some(1.0))
        .count();

    let mut left = vec![Span::styled("──", Style::new().fg(edge))];
    left.extend(bracketed(edge, vec![Span::styled("mattertui", t.bold(t.title()))]));
    left.push(Span::styled("─", Style::new().fg(edge)));
    left.extend(bracketed(
        edge,
        vec![
            Span::styled(format!("{live}/{total}"), t.bold(if live == total { t.good() } else { t.warn() })),
            Span::styled(" live", t.muted()),
        ],
    ));
    left.push(Span::styled("─", Style::new().fg(edge)));
    left.extend(bracketed(
        edge,
        vec![
            Span::styled("lights ", t.muted()),
            Span::styled(lights_on.to_string(), t.bold(if lights_on > 0 { super::glow(app) } else { t.dim() })),
        ],
    ));
    let unseen = app.unseen_warnings();
    if unseen > 0 && !app.log_open {
        left.push(Span::styled("─", Style::new().fg(edge)));
        left.extend(bracketed(edge, vec![Span::styled(format!("{unseen} warnings"), t.bold(t.warn()))]));
    }
    let left = Line::from(left);
    let left_w = left.width() as u16;
    left.render(area, buf);

    let mut right = bracketed(edge, vec![Span::styled(app.range_label(), t.bold(t.title()))]);
    right.push(Span::styled("─", Style::new().fg(edge)));
    right.extend(bracketed(edge, vec![Span::styled("?", t.bold(t.hi())), Span::styled("help", t.title())]));
    right.push(Span::styled("──", Style::new().fg(edge)));
    let right = Line::from(right);
    let right_w = right.width() as u16;
    right.alignment(Alignment::Right).render(area, buf);

    // The clock sits in the middle when there's room for it.
    let clock = Line::from(bracketed(
        edge,
        vec![Span::styled(jiff::ZonedDateTime::now().strftime("%H:%M:%S").to_string(), t.bold(t.title()))],
    ));
    let cw = clock.width() as u16;
    let cx = area.x + area.width.saturating_sub(cw) / 2;
    if cx > area.x + left_w + 1 && cx + cw + 1 < area.right().saturating_sub(right_w) {
        clock.render(Rect::new(cx, area.y, cw, 1), buf);
    }
}

/// The log, in its own box.
pub fn log_panel(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let lines = app.logs.snapshot();
    let colour = if app.log_focus { t.title() } else { t.dim() };
    let left = vec![
        Span::styled("⁰", t.bold(t.hi())),
        Span::styled("log", t.bold(t.title())),
    ];
    let right = if app.log_scroll == 0 {
        vec![Span::styled("Tab", t.bold(t.hi())), Span::styled(" scroll", t.muted())]
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
            spans.push(Span::styled(format!("{dev} "), t.bold(colour)));
        } else if l.target != "mattertui" {
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
    let inner = btop_box(app, r, buf, t.title(), vec![Span::styled("help", t.bold(t.title()))], vec![]);
    for (i, (k, v)) in HELP.iter().enumerate() {
        Line::from(vec![
            Span::styled(format!(" {k:<12}"), t.bold(t.hi())),
            Span::styled(*v, t.text()),
        ])
        .render(Rect::new(inner.x, inner.y + 1 + i as u16, inner.width, 1), buf);
    }
}

/// The bottom rule carries recent events, newest first:
/// `──┘events└─ Lamp on 2m · Bedside 18.0° ↑ 5m ─────`. It scrolls like a
/// marquee when it doesn't fit (and motion is on). Returns whether it's
/// scrolling.
pub fn ticker(app: &App, area: Rect, buf: &mut Buffer) -> bool {
    use crate::ui::motion::Tone;
    let t = &app.theme;
    let edge = t.frame();
    if area.width < 12 {
        return false;
    }
    rule(area, buf, edge);
    let label = Line::from(vec![
        Span::styled("──┘", Style::new().fg(edge)),
        Span::styled("events", t.bold(t.title())),
        Span::styled("└─ ", Style::new().fg(edge)),
    ]);
    let lw = label.width() as u16;
    label.render(area, buf);
    let text_area = Rect::new(area.x + lw, area.y, area.width.saturating_sub(lw + 2), 1);

    if app.ticker.is_empty() {
        Line::styled("watching for changes… ", t.muted()).render(text_area, buf);
        return false;
    }
    let mut spans = Vec::new();
    for (n, e) in app.ticker.events().enumerate() {
        if n > 0 {
            spans.push(Span::styled(" · ", t.muted()));
        }
        let colour = match e.tone {
            Tone::Normal => app
                .rooms
                .iter()
                .position(|r| r.contains(&e.device))
                .map_or(t.title(), |room| super::room::accent(app, room)),
            Tone::Glow => t.glow(),
            Tone::Heat => t.heat(),
            Tone::Good => t.good(),
            Tone::Bad => t.bad(),
        };
        spans.push(Span::styled(e.text.clone(), t.bold(colour)));
        spans.push(Span::styled(format!(" {}", super::ago(e.at)), t.muted()));
    }
    spans.push(Span::raw(" "));
    let line = Line::from(spans);
    let width = line.width() as u16;
    if width <= text_area.width || !app.motion {
        line.render(text_area, buf);
        return false;
    }

    // Marquee: render the line plus a gap once, then copy a window that
    // advances with time, wrapping around.
    const GAP: u16 = 8;
    const COLS_PER_SEC: f64 = 7.0;
    let period = width + GAP;
    let mut strip = Buffer::empty(Rect::new(0, 0, period, 1));
    line.render(Rect::new(0, 0, width, 1), &mut strip);
    let offset = (app.epoch.elapsed().as_secs_f64() * COLS_PER_SEC) as u64 % u64::from(period);
    for x in 0..text_area.width {
        let src = ((offset + u64::from(x)) % u64::from(period)) as u16;
        buf[(text_area.x + x, text_area.y)] = strip[(src, 0)].clone();
    }
    true
}

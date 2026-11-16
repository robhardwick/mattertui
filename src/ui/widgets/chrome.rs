//! Top bar, log panel and help overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use tracing::Level;

use super::current;
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;
use crate::ui::icons;

/// A btop-style rule across the top:
/// `──┤ mattertui ├─┤● 5/6 live · 💡 2 on├────┤ 00:12 ├────┤1h · ? help├──`.
pub fn top_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let total = app.devices.len();
    let live = (0..total).filter(|&i| app.health(i).is_ok()).count();
    let lights_on = (0..total)
        .filter(|&i| app.config.devices[i].kind == DeviceKind::Plug && current(&app.devices[i], Metric::On) == Some(1.0))
        .count();

    let rule = if t.ascii { '-' } else { '─' };
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_char(rule).set_style(Style::new().fg(t.frame()));
    }
    let open = || Span::styled("┤", Style::new().fg(t.frame()));
    let close = || Span::styled("├", Style::new().fg(t.frame()));
    let sep = || Span::styled(" · ", Style::new().fg(t.faint()));

    let mut left = vec![
        Span::styled("──", Style::new().fg(t.frame())),
        open(),
        Span::styled(" mattertui ", t.bold(t.title())),
        close(),
        Span::styled("─", Style::new().fg(t.frame())),
        open(),
        Span::styled(" ● ", Style::new().fg(if live == total { t.good() } else { t.warn() })),
        Span::styled(format!("{live}/{total} live"), t.muted()),
        sep(),
        Span::styled(format!("{} ", icons::cell("lamp", t.ascii)), Style::new().fg(if lights_on > 0 { super::glow(app) } else { t.dim() })),
        Span::styled(format!("{lights_on} on "), t.muted()),
    ];
    let unseen = app.unseen_warnings();
    if unseen > 0 && !app.log_open {
        left.push(Span::styled(format!("· ⚠ {unseen} "), t.bold(t.warn())));
    }
    left.push(close());
    let left = Line::from(left);
    let left_w = left.width() as u16;
    left.render(area, buf);

    let right = Line::from(vec![
        open(),
        Span::styled(format!(" {} ", app.range_label()), t.muted()),
        Span::styled("· ", Style::new().fg(t.faint())),
        Span::styled("?", t.bold(t.orange())),
        Span::styled(" help ", t.muted()),
        close(),
        Span::styled("──", Style::new().fg(t.frame())),
    ]);
    let right_w = right.width() as u16;
    right.alignment(Alignment::Right).render(area, buf);

    // The clock sits in the middle when there's room for it.
    let clock = Line::from(vec![
        open(),
        Span::styled(format!(" {} ", jiff::ZonedDateTime::now().strftime("%H:%M")), t.bold(t.fg())),
        close(),
    ]);
    let cw = clock.width() as u16;
    let cx = area.x + area.width.saturating_sub(cw) / 2;
    if cx > area.x + left_w + 1 && cx + cw + 1 < area.right().saturating_sub(right_w) {
        clock.render(Rect::new(cx, area.y, cw, 1), buf);
    }
}

pub fn log_panel(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let lines = app.logs.snapshot();
    let status = if app.log_scroll == 0 {
        Span::styled(" following ", t.muted())
    } else {
        Span::styled(format!(" paused ↑{} · End to follow ", app.log_scroll), Style::new().fg(t.warn()))
    };
    let border = if app.log_focus { Style::new().fg(t.title()) } else { Style::new().fg(t.frame()) };
    let block = Block::new()
        .borders(Borders::TOP)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(vec![
            Span::styled(" log ", t.bold(t.fg())),
            Span::styled(if app.log_focus { "(Tab to leave) " } else { "(Tab to scroll) " }, Style::new().fg(t.faint())),
        ]))
        .title(Line::from(status).alignment(Alignment::Right));
    let inner = block.inner(area);
    block.render(area, buf);

    let height = inner.height as usize;
    let end = lines.len().saturating_sub(app.log_scroll);
    let start = end.saturating_sub(height);
    for (row, l) in lines[start..end].iter().enumerate() {
        let level_colour = match l.level {
            Level::ERROR => t.bad(),
            Level::WARN => t.warn(),
            Level::INFO => t.cool(),
            _ => t.dim(),
        };
        let mut spans = vec![
            Span::styled(format!("{} ", l.time), Style::new().fg(t.faint())),
            Span::styled(format!("{:<5} ", l.level.as_str()), Style::new().fg(level_colour)),
        ];
        if let Some(dev) = &l.device {
            let colour = device_accent(app, dev).unwrap_or(t.dim());
            spans.push(Span::styled(format!("{dev} "), Style::new().fg(colour)));
        } else if l.target != "mattertui" {
            spans.push(Span::styled(format!("{} ", l.target), Style::new().fg(t.faint())));
        }
        spans.push(Span::styled(l.message.clone(), t.text()));
        if !l.fields.is_empty() {
            spans.push(Span::styled(format!(" {}", l.fields), t.muted()));
        }
        Line::from(spans).render(Rect::new(inner.x, inner.y + row as u16, inner.width, 1), buf);
    }
}

fn device_accent(app: &App, id: &str) -> Option<ratatui::style::Color> {
    let i = app.config.devices.iter().position(|d| d.id.get_ref() == id)?;
    let room = app.rooms.iter().position(|r| r.contains(&i))?;
    Some(super::room::accent(app, room))
}

const HELP: &[(&str, &str)] = &[
    ("←↓↑→ / hjkl", "move between devices"),
    ("1–9", "jump to a room"),
    ("Enter", "device detail (Esc to go back)"),
    ("[ ]", "chart range"),
    ("m", "toggle motion"),
    ("p", "pair the selected device"),
    ("U", "unpair the selected device"),
    ("`", "toggle log panel"),
    ("Tab", "focus log panel (↑↓ PgUp PgDn End)"),
    ("?", "this help"),
    ("q", "quit"),
];

pub fn help(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let w = 52.min(area.width);
    let h = (HELP.len() as u16 + 4).min(area.height);
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    Clear.render(r, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(t.fg()))
        .title(Line::styled(" keys ", t.bold(t.fg())));
    let inner = block.inner(r);
    block.render(r, buf);
    for (i, (k, v)) in HELP.iter().enumerate() {
        Line::from(vec![
            Span::styled(format!(" {k:<14}"), t.bold(t.orange())),
            Span::styled(*v, t.text()),
        ])
        .render(Rect::new(inner.x, inner.y + 1 + i as u16, inner.width, 1), buf);
    }
}

/// One line of recent events, newest first. Scrolls like a marquee when it
/// doesn't fit (and motion is on). Returns whether it is scrolling.
pub fn ticker(app: &App, area: Rect, buf: &mut Buffer) -> bool {
    use crate::ui::motion::Tone;
    let t = &app.theme;
    if area.width == 0 {
        return false;
    }
    if app.ticker.is_empty() {
        Line::styled(" watching for changes…", Style::new().fg(t.faint())).render(area, buf);
        return false;
    }
    let mut spans = vec![Span::raw(" ")];
    for (n, e) in app.ticker.events().enumerate() {
        if n > 0 {
            spans.push(Span::styled("   ·   ", Style::new().fg(t.faint())));
        }
        let cfg = &app.config.devices[e.device];
        let glyph = icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), icons::default_for(cfg.kind), t.ascii);
        let colour = match e.tone {
            Tone::Normal => t.fg(),
            Tone::Glow => t.glow(),
            Tone::Heat => t.heat(),
            Tone::Good => t.good(),
            Tone::Bad => t.bad(),
        };
        spans.push(Span::raw(format!("{glyph} ")));
        spans.push(Span::styled(e.text.clone(), Style::new().fg(colour)));
        spans.push(Span::styled(format!(" {}", super::ago(e.at)), Style::new().fg(t.faint())));
    }
    let line = Line::from(spans);
    let width = line.width() as u16;
    if width <= area.width || !app.motion {
        line.render(area, buf);
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
    for x in 0..area.width {
        let src = ((offset + u64::from(x)) % u64::from(period)) as u16;
        let mut cell = strip[(src, 0)].clone();
        // A wide glyph in the last column would spill off the line.
        if x == area.width - 1 && Span::raw(cell.symbol()).width() > 1 {
            cell.set_symbol(" ");
        }
        buf[(area.x + x, area.y)] = cell;
    }
    true
}

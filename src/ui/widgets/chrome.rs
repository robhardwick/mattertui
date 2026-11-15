//! Top bar, log panel and help overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use tracing::Level;

use super::current;
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;

pub fn top_bar(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let total = app.devices.len();
    let live = (0..total).filter(|&i| app.health(i).is_ok()).count();
    let lights_on = (0..total)
        .filter(|&i| app.config.devices[i].kind == DeviceKind::Plug && current(&app.devices[i], Metric::On) == Some(1.0))
        .count();

    let mut left = vec![
        Span::styled(" mattertui ", t.bold(t.fg())),
        Span::styled("│ ", Style::new().fg(t.faint())),
        Span::styled("● ", Style::new().fg(if live == total { t.good() } else { t.warn() })),
        Span::styled(format!("{live}/{total} live"), t.muted()),
        Span::styled("  ", t.muted()),
        Span::styled(if t.ascii { "* " } else { "☼ " }, Style::new().fg(if lights_on > 0 { t.glow() } else { t.dim() })),
        Span::styled(format!("{lights_on} on"), t.muted()),
    ];
    let unseen = app.unseen_warnings();
    if unseen > 0 && !app.log_open {
        left.push(Span::styled(format!("  ⚠ {unseen} "), Style::new().fg(t.warn()).add_modifier(Modifier::BOLD)));
        left.push(Span::styled("(` for log)", Style::new().fg(t.faint())));
    }
    Line::from(left).render(area, buf);

    let clock = jiff::ZonedDateTime::now().strftime("%H:%M").to_string();
    Line::from(vec![
        Span::styled(format!("{} ", app.range_label()), t.muted()),
        Span::styled("? help ", Style::new().fg(t.faint())),
        Span::styled(format!("{clock} "), t.bold(t.fg())),
    ])
    .alignment(Alignment::Right)
    .render(area, buf);
}

pub fn log_panel(app: &App, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let lines = app.logs.snapshot();
    let status = if app.log_scroll == 0 {
        Span::styled(" following ", t.muted())
    } else {
        Span::styled(format!(" paused ↑{} · End to follow ", app.log_scroll), Style::new().fg(t.warn()))
    };
    let border = if app.log_focus { Style::new().fg(t.fg()) } else { Style::new().fg(t.border()) };
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
    ("Enter", "device detail (Esc to go back)"),
    ("[ ]", "chart range"),
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
            Span::styled(format!(" {k:<14}"), t.bold(t.glow())),
            Span::styled(*v, t.text()),
        ])
        .render(Rect::new(inner.x, inner.y + 1 + i as u16, inner.width, 1), buf);
    }
}

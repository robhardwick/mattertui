//! Pair / unpair dialog.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget, Wrap};

use crate::ui::app::{App, Modal, ModalKind, Phase};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

pub fn render(app: &App, m: &Modal, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let name = &app.config.devices[m.device].name;
    let room = app.rooms.iter().position(|r| r.contains(&m.device)).unwrap_or(0);
    let accent = super::room::accent(app, room);

    let w = 64.min(area.width);
    let h = 11.min(area.height);
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    Clear.render(r, buf);
    let title = match m.kind {
        ModalKind::Pair => format!("Pair {name}"),
        ModalKind::Unpair => format!("Unpair {name}"),
    };
    let hint = match &m.phase {
        Phase::Input { .. } if m.kind == ModalKind::Pair => "Enter pair · Esc cancel",
        Phase::Input { .. } => "y unpair · n cancel",
        Phase::Working { .. } => "Esc hide (keeps going)",
        Phase::Done { .. } => "Enter close",
    };
    let edge = Style::new().fg(accent);
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(edge)
        .title(Line::from(vec![
            Span::styled("─┐", edge),
            Span::styled(title, t.bold(t.title())),
            Span::styled("┌", edge),
        ]))
        .title_bottom(
            Line::from(vec![Span::styled("┘", edge), Span::styled(hint, t.muted()), Span::styled("└─", edge)])
                .alignment(Alignment::Right),
        );
    let inner = block.inner(r);
    block.render(r, buf);
    let inner = Rect::new(inner.x + 2, inner.y + 1, inner.width.saturating_sub(4), inner.height.saturating_sub(1));
    let row = |n: u16| Rect::new(inner.x, inner.y + n, inner.width, 1).intersection(inner);

    match (&m.phase, m.kind) {
        (Phase::Input { error }, ModalKind::Pair) => {
            Line::from(vec![
                Span::styled("1. ", Style::new().fg(accent)),
                Span::styled("Home app → the device → Settings → ", t.text()),
                Span::styled("Turn On Pairing Mode", t.bold(t.fg())),
            ])
            .render(row(0), buf);
            Line::from(vec![
                Span::styled("2. ", Style::new().fg(accent)),
                Span::styled("Type the setup code it shows, then Enter", t.text()),
            ])
            .render(row(1), buf);

            let cursor = if t.ascii { "_" } else { "▏" };
            Line::from(vec![
                Span::styled("› ", Style::new().fg(accent).add_modifier(Modifier::BOLD)),
                Span::styled(m.input.clone(), t.bold(t.fg())),
                Span::styled(cursor, Style::new().fg(accent)),
            ])
            .render(row(3), buf);
            let (hint, colour) = code_hint(&m.input);
            Line::styled(hint, Style::new().fg(match colour {
                HintColour::Muted => t.dim(),
                HintColour::Good => t.good(),
                HintColour::Bad => t.bad(),
            }))
            .alignment(Alignment::Right)
            .render(row(3), buf);
            // Underline the input.
            Line::styled("─".repeat(inner.width as usize), Style::new().fg(t.faint())).render(row(4), buf);

            if let Some(e) = error {
                ratatui::widgets::Paragraph::new(Line::styled(e.clone(), Style::new().fg(t.bad())))
                    .wrap(Wrap { trim: true })
                    .render(Rect::new(inner.x, inner.y + 5, inner.width, 3).intersection(inner), buf);
            }
        }
        (Phase::Input { .. }, ModalKind::Unpair) => {
            Line::styled(format!("Remove mattertui from {name}?"), t.bold(t.fg())).render(row(0), buf);
            Line::styled("The device stays in Apple Home; only this app's access is removed.", t.muted())
                .render(row(2), buf);
            Line::styled("You'll need a new setup code to pair it again.", t.muted()).render(row(3), buf);
        }
        (Phase::Working { since }, kind) => {
            let frames: &[&str] = if t.ascii { &SPINNER_ASCII } else { &SPINNER };
            let frame = frames[(since.elapsed().as_millis() / 100) as usize % frames.len()];
            let verb = if kind == ModalKind::Pair { "Pairing" } else { "Unpairing" };
            Line::from(vec![
                Span::styled(format!("{frame} "), Style::new().fg(accent).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{verb}…"), t.bold(t.fg())),
                Span::styled(format!("  {}s", since.elapsed().as_secs()), t.muted()),
            ])
            .render(row(0), buf);
            if kind == ModalKind::Pair {
                Line::styled("This usually takes 10–30 seconds.", t.muted()).render(row(1), buf);
            }
            // Latest log line as live progress.
            if let Some(l) = app.logs.last() {
                let text = format!("{} {}", l.message, l.fields);
                ratatui::widgets::Paragraph::new(Line::styled(text, Style::new().fg(t.faint())))
                    .wrap(Wrap { trim: true })
                    .render(Rect::new(inner.x, inner.y + 3, inner.width, 4).intersection(inner), buf);
            }
        }
        (Phase::Done { ok, message }, _) => {
            let (mark, colour) = if *ok { ("✓", t.good()) } else { ("✗", t.bad()) };
            Line::from(vec![
                Span::styled(format!("{mark} "), t.bold(colour)),
                Span::styled(if *ok { "Done" } else { "Couldn't do that" }, t.bold(t.fg())),
            ])
            .render(row(0), buf);
            ratatui::widgets::Paragraph::new(Line::styled(message.clone(), t.text()))
                .wrap(Wrap { trim: true })
                .render(Rect::new(inner.x, inner.y + 2, inner.width, 5).intersection(inner), buf);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HintColour {
    Muted,
    Good,
    Bad,
}

/// Feedback while typing a code.
fn code_hint(input: &str) -> (String, HintColour) {
    if input.trim().is_empty() {
        return (String::new(), HintColour::Muted);
    }
    if input.trim_start().starts_with("MT:") {
        return ("QR code text".into(), HintColour::Muted);
    }
    let digits = input.chars().filter(char::is_ascii_digit).count();
    if input.chars().any(|c| c.is_ascii_alphabetic()) {
        return ("digits only".into(), HintColour::Bad);
    }
    match digits {
        n if n < 11 => (format!("{n}/11"), HintColour::Muted),
        11 | 21 => match crate::matter::pairing::normalize_code(input) {
            Ok(_) => ("✓ valid".into(), HintColour::Good),
            Err(_) => ("check digit wrong".into(), HintColour::Bad),
        },
        n if n < 21 => (format!("{n}/21"), HintColour::Muted),
        _ => ("too long".into(), HintColour::Bad),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_while_typing() {
        assert_eq!(code_hint("3497-01"), ("6/11".into(), HintColour::Muted));
        assert_eq!(code_hint("3497-011-2332"), ("✓ valid".into(), HintColour::Good));
        assert_eq!(code_hint("3497-011-2333").1, HintColour::Bad);
        assert_eq!(code_hint("MT:Y.K9").1, HintColour::Muted);
        assert_eq!(code_hint("12a").1, HintColour::Bad);
    }
}

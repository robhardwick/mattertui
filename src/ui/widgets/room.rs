//! Room box: a tinted rounded frame with a btop-style title
//! (`╭─┤¹ 🏠 Living room├──────┤20.5° · 51%├─╮`). Devices are borderless
//! sections inside it.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use super::{current, fmt_temp};
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;
use crate::ui::icons;
use crate::ui::motion::mix;

const SUPERSCRIPT: [&str; 9] = ["¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"];

/// The room's tint (teal family).
pub fn accent(app: &App, room: usize) -> Color {
    let cfg = &app.config.rooms[room];
    app.theme.accent(cfg.accent.as_ref().map(|a| a.get_ref().as_str()), room)
}

pub fn render(app: &App, room: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.rooms[room];
    let tint = accent(app, room);
    let devices = &app.rooms[room];
    let has_selection = devices.contains(&app.selected);
    // Frames sit back unless the room holds the selection.
    let frame = if has_selection { tint } else { mix(t.frame(), tint, 0.35) };
    let bracket = |s: &'static str| Span::styled(s, Style::new().fg(frame));

    let mut title = vec![bracket("┤")];
    if let Some(n) = SUPERSCRIPT.get(room) {
        title.push(Span::styled(*n, t.bold(t.orange())));
    }
    if let Some(icon) = cfg.icon.as_ref().and_then(|i| icons::resolve(i.get_ref(), t.ascii)) {
        title.push(Span::raw(format!(" {}", icons::pad2(icon))));
    }
    title.push(Span::styled(format!(" {} ", cfg.name), t.bold(if has_selection { t.title() } else { tint })));
    title.push(bracket("├"));

    let title = Line::from(title);
    let room_for_summary = area.width.saturating_sub(title.width() as u16 + 6) as usize;
    let mut block = Block::bordered()
        .border_type(if t.ascii { BorderType::Plain } else { BorderType::Rounded })
        .border_style(Style::new().fg(frame))
        .title(title);
    // The name always wins: drop summary items from the end until it fits.
    let mut summary = summary(app, devices);
    while !summary.is_empty() && Line::from(summary.clone()).width() > room_for_summary {
        summary.truncate(summary.len().saturating_sub(3)); // a value and its separators
        while summary.last().is_some_and(|s| s.content.trim() == "·" || s.content.trim().is_empty()) {
            summary.pop();
        }
        if summary.len() <= 1 {
            summary.clear();
        } else {
            summary.push(Span::raw(" "));
        }
    }
    if !summary.is_empty() {
        let mut spans = vec![bracket("┤")];
        spans.extend(summary);
        spans.push(bracket("├"));
        block = block.title(Line::from(spans).alignment(Alignment::Right));
    }
    block.render(area, buf);
}

/// e.g. " 19.8° · 45% · 💡 1/2 "
fn summary(app: &App, devices: &[usize]) -> Vec<Span<'static>> {
    let t = &app.theme;
    let mean = |m: Metric| {
        let v: Vec<f64> = devices.iter().filter_map(|&i| app.shown(i, m)).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    let mut parts: Vec<Span<'static>> = Vec::new();
    let sep = |p: &mut Vec<Span<'static>>| {
        if !p.is_empty() {
            p.push(Span::styled(" · ", Style::new().fg(t.faint())));
        }
    };
    if let Some(c) = mean(Metric::Temperature) {
        parts.push(Span::styled(fmt_temp(app, c), Style::new().fg(t.temperature(c))));
    }
    if let Some(h) = mean(Metric::Humidity) {
        sep(&mut parts);
        parts.push(Span::styled(format!("{h:.0}%"), Style::new().fg(t.humidity(h))));
    }
    let plugs: Vec<usize> = devices
        .iter()
        .copied()
        .filter(|&i| app.config.devices[i].kind == DeviceKind::Plug)
        .collect();
    if !plugs.is_empty() {
        let on = plugs.iter().filter(|&&i| current(&app.devices[i], Metric::On) == Some(1.0)).count();
        sep(&mut parts);
        let glyph = icons::cell("lamp", t.ascii);
        let colour = if on > 0 { super::glow(app) } else { t.dim() };
        parts.push(Span::styled(format!("{glyph} {on}/{}", plugs.len()), Style::new().fg(colour)));
    }
    let heating = devices.iter().any(|&i| {
        let d = &app.devices[i];
        current(d, Metric::Running).is_some_and(|r| r as u64 & 1 != 0) || current(d, Metric::HeatDemand).is_some_and(|p| p > 0.0)
    });
    if heating {
        sep(&mut parts);
        parts.push(Span::styled(icons::cell("fire", t.ascii), Style::new().fg(super::flame(app))));
    }
    if parts.is_empty() {
        return parts;
    }
    parts.insert(0, Span::raw(" "));
    parts.push(Span::raw(" "));
    parts
}

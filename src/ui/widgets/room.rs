//! Room panel: accent border, name, and a summary of its devices.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use super::{current, fmt_temp};
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;
use crate::ui::icons;

pub fn accent(app: &App, room: usize) -> Color {
    let cfg = &app.config.rooms[room];
    app.theme.accent(cfg.accent.as_ref().map(|a| a.get_ref().as_str()), room)
}

pub fn render(app: &App, room: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.rooms[room];
    let accent = accent(app, room);
    let devices = &app.rooms[room];
    let has_selection = devices.contains(&app.selected);

    let mut title = vec![Span::raw(" ")];
    if let Some(icon) = cfg.icon.as_ref().and_then(|i| icons::resolve(i.get_ref(), t.ascii)) {
        title.push(Span::styled(format!("{} ", icons::pad2(icon)), Style::new().fg(accent)));
    }
    title.push(Span::styled(cfg.name.clone(), Style::new().fg(accent).add_modifier(Modifier::BOLD)));
    title.push(Span::raw(" "));

    let border = if has_selection {
        Style::new().fg(accent)
    } else {
        Style::new().fg(accent).add_modifier(Modifier::DIM)
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(title))
        .title(Line::from(summary(app, devices)).alignment(Alignment::Right))
        .render(area, buf);
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
    if parts.is_empty() {
        return parts;
    }
    parts.insert(0, Span::raw(" "));
    parts.push(Span::raw(" "));
    parts
}

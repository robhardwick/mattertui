//! Room box: btop style, in the room's own colour, titled
//! `┌─┐¹Living room┌──────┐20.5° 51% lights 1/1┌─┐`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::{btop_box, current, fmt_temp};
use crate::config::DeviceKind;
use crate::state::Metric;
use crate::ui::app::App;
use crate::ui::icons;

const SUPERSCRIPT: [&str; 9] = ["¹", "²", "³", "⁴", "⁵", "⁶", "⁷", "⁸", "⁹"];

/// The room's box colour.
pub fn accent(app: &App, room: usize) -> Color {
    let cfg = &app.config.rooms[room];
    app.theme.accent(cfg.accent.as_ref().map(|a| a.get_ref().as_str()), room)
}

pub fn render(app: &App, room: usize, area: Rect, buf: &mut Buffer) {
    let t = &app.theme;
    let cfg = &app.config.rooms[room];
    let colour = accent(app, room);
    let devices = &app.rooms[room];

    let mut title = Vec::new();
    if let Some(n) = SUPERSCRIPT.get(room) {
        title.push(Span::styled(*n, t.bold(t.hi())));
    }
    if let Some(g) = icons::glyph(cfg.icon.as_ref().map(|i| i.get_ref().as_str()), None, t.ascii) {
        title.push(Span::styled(format!("{g} "), Style::new().fg(colour)));
    }
    title.push(Span::styled(cfg.name.clone(), t.bold(t.title())));

    // The name always wins: drop summary items from the end until it fits.
    let room_for_summary = area.width.saturating_sub(Line::from(title.clone()).width() as u16 + 10) as usize;
    let mut items = summary(app, devices);
    while !items.is_empty() && joined_width(&items) > room_for_summary {
        items.pop();
    }
    let mut right = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            right.push(Span::raw(" "));
        }
        right.extend(item);
    }
    btop_box(app, area, buf, colour, title, right);
}

fn joined_width(items: &[Vec<Span<'static>>]) -> usize {
    items.iter().map(|i| Line::from(i.clone()).width()).sum::<usize>() + items.len().saturating_sub(1)
}

/// Summary items, most important first: mean temperature, mean humidity,
/// lights on, heating.
fn summary(app: &App, devices: &[usize]) -> Vec<Vec<Span<'static>>> {
    let t = &app.theme;
    let mean = |m: Metric| {
        let v: Vec<f64> = devices.iter().filter_map(|&i| app.shown(i, m)).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    let mut items = Vec::new();
    if let Some(c) = mean(Metric::Temperature) {
        items.push(vec![Span::styled(fmt_temp(app, c), t.bold(t.title()))]);
    }
    if let Some(h) = mean(Metric::Humidity) {
        items.push(vec![
            Span::styled(format!("{} ", icons::get("humidity", t.ascii)), t.muted()),
            Span::styled(format!("{h:.0}%"), t.bold(t.title())),
        ]);
    }
    let plugs: Vec<usize> = devices.iter().copied().filter(|&i| app.config.devices[i].kind == DeviceKind::Plug).collect();
    if !plugs.is_empty() {
        let on = plugs.iter().filter(|&&i| current(&app.devices[i], Metric::On) == Some(1.0)).count();
        items.push(vec![
            Span::styled(format!("{} ", icons::get("lamp", t.ascii)), if on > 0 { Style::new().fg(super::glow(app)) } else { t.muted() }),
            Span::styled(format!("{on}/{}", plugs.len()), t.bold(if on > 0 { super::glow(app) } else { t.dim() })),
        ]);
    }
    let heating = devices.iter().any(|&i| {
        let d = &app.devices[i];
        current(d, Metric::Running).is_some_and(|r| r as u64 & 1 != 0) || current(d, Metric::HeatDemand).is_some_and(|p| p > 0.0)
    });
    if heating {
        items.push(vec![Span::styled(format!("{} heating", icons::get("heating", t.ascii)), Style::new().fg(super::flame(app)))]);
    }
    items
}

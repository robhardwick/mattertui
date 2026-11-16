//! Rendering. Every function draws into a `Buffer` so the dashboard can be
//! rendered off-screen and scrolled.

pub mod chrome;
pub mod detail;
pub mod modal;
pub mod room;
pub mod tile;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Widget;

use super::app::{App, Health};
use super::graph::{self, Graph};
use super::{motion, series};
use crate::config::TemperatureUnit;
use crate::state::{DeviceState, History, Metric};

/// "21.5°" in the configured unit.
pub fn fmt_temp(app: &App, celsius: f64) -> String {
    fmt_temp_unit(app.config.ui.temperature_unit, celsius)
}

pub fn fmt_temp_unit(unit: TemperatureUnit, celsius: f64) -> String {
    match unit {
        TemperatureUnit::Celsius => format!("{celsius:.1}°"),
        TemperatureUnit::Fahrenheit => format!("{:.1}°", celsius * 9.0 / 5.0 + 32.0),
    }
}

pub fn current(d: &DeviceState, m: Metric) -> Option<f64> {
    d.current.get(&m).copied().flatten()
}

/// ↑ ↓ → comparing now with ~15 minutes ago.
pub fn trend(h: Option<&History>, now: f64, now_min: i64) -> &'static str {
    let Some(h) = h else { return "" };
    let then = h.buckets().filter(|b| b.minute <= now_min - 15).last();
    match then.map(|b| now - b.last) {
        Some(d) if d > 0.2 => "↑",
        Some(d) if d < -0.2 => "↓",
        Some(_) => "→",
        None => "",
    }
}

/// Warm light that gently breathes while motion is on.
pub fn glow(app: &App) -> Color {
    let t = &app.theme;
    if !app.motion {
        return t.glow();
    }
    let b = motion::breathe(app.epoch, Duration::from_millis(3200), Instant::now());
    motion::mix(t.glow(), t.glow_bright(), b * 0.7)
}

/// Flickering flame colour for active heating.
pub fn flame(app: &App) -> Color {
    let t = &app.theme;
    if !app.motion {
        return t.heat();
    }
    let now = Instant::now();
    // Two incommensurate waves read as an irregular flicker.
    let f = 0.65 * motion::breathe(app.epoch, Duration::from_millis(1300), now)
        + 0.35 * motion::breathe(app.epoch, Duration::from_millis(370), now);
    motion::mix(t.heat(), t.glow(), f)
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner(app: &App) -> &'static str {
    if app.theme.ascii || !app.motion {
        return "…";
    }
    SPINNER[(app.epoch.elapsed().as_millis() / 90) as usize % SPINNER.len()]
}

pub fn health_badge(app: &App, h: Health) -> Option<(String, Color)> {
    let t = &app.theme;
    Some(match h {
        Health::Live => return None,
        Health::Unpaired => ("not paired".into(), t.dim()),
        Health::Connecting => (format!("{} connecting", spinner(app)), t.dim()),
        Health::Stale => ("stale".into(), t.warn()),
        Health::Reconnecting => ("↻".into(), t.faint()),
        Health::Offline => ("offline".into(), t.bad()),
    })
}

/// One sample per graph column (or per braille dot column).
type Samples = Vec<Option<f64>>;

/// Samples for `metric` (and an overlay metric) across `width` cells.
fn graph_samples(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(Samples, Samples)> {
    let d = &app.devices[device];
    let h = d.history.get(&metric)?;
    let now = crate::state::unix_minute();
    let n = Graph::samples_for(width, app.theme.ascii);
    let samples = series::sample(h, now, app.range_minutes(), n);
    let over = overlay
        .filter(|_| d.current.get(&Metric::Mode).copied().flatten() != Some(0.0))
        .and_then(|m| d.history.get(&m))
        .map(|h| series::sample(h, now, app.range_minutes(), n))
        .unwrap_or_default();
    Some((samples, over))
}

/// The value range a graph of `metric` uses at this width.
pub fn metric_range(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(f64, f64)> {
    let (samples, over) = graph_samples(app, device, metric, overlay, width)?;
    if metric == Metric::On {
        // Fixed scale so "on" always reaches the top.
        return samples.iter().any(Option::is_some).then_some((0.0, 1.0));
    }
    let min_span = match metric {
        Metric::Temperature | Metric::HeatSetpoint => 1.5,
        _ => 6.0,
    };
    graph::range(samples.iter().chain(&over).flatten().copied(), min_span)
}

/// The graph alone: gradient braille fill with an optional overlay line.
pub fn metric_graph_bare(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, area: Rect, buf: &mut Buffer) -> bool {
    let t = &app.theme;
    if area.is_empty() {
        return false;
    }
    let Some((samples, over)) = graph_samples(app, device, metric, overlay, area.width) else {
        // Only a live device is "collecting"; an offline one just has no data.
        if app.health(device).is_ok() {
            Line::styled("collecting history…", Style::new().fg(t.faint()))
                .render(Rect::new(area.x, area.y + area.height - 1, area.width, 1), buf);
        }
        return false;
    };
    let Some((lo, hi)) = metric_range(app, device, metric, overlay, area.width) else {
        return false;
    };
    let glow = glow(app);
    let colour = |v: f64| match metric {
        Metric::Temperature => t.temperature(v),
        Metric::Humidity => t.humidity(v),
        Metric::On => glow,
        _ => t.graph((v - lo) / (hi - lo)),
    };
    Graph {
        samples: &samples,
        lo,
        hi,
        colour: &colour,
        overlay: (!over.is_empty()).then_some((over.as_slice(), t.heat())),
        ascii: t.ascii,
    }
    .render(area, buf);
    true
}

/// A tile graph: the bare graph plus faint scale labels at the right edge.
pub fn metric_graph(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, area: Rect, buf: &mut Buffer) {
    if !metric_graph_bare(app, device, metric, overlay, area, buf) {
        return;
    }
    if area.height >= 3
        && area.width >= 20
        && let Some((lo, hi)) = metric_range(app, device, metric, overlay, area.width)
    {
        let fmt = |v: f64| match metric {
            Metric::Temperature => fmt_temp(app, v),
            _ => format!("{v:.0}%"),
        };
        let style = Style::new().fg(app.theme.dim());
        Line::styled(fmt(hi), style).alignment(Alignment::Right).render(Rect::new(area.x, area.y, area.width, 1), buf);
        Line::styled(fmt(lo), style)
            .alignment(Alignment::Right)
            .render(Rect::new(area.x, area.bottom() - 1, area.width, 1), buf);
    }
}

/// A one-row on/off timeline (plugs, heating) over the selected range.
pub fn timeline(app: &App, h: Option<&History>, area: Rect, buf: &mut Buffer, on: Color) {
    let Some(h) = h else { return };
    let now = crate::state::unix_minute();
    let values = series::sample(h, now, app.range_minutes(), area.width as usize);
    let (on_g, off_g) = if app.theme.ascii { ('=', '-') } else { ('━', '─') };
    for (i, v) in values.iter().enumerate() {
        let (g, c) = match v {
            Some(v) if *v != 0.0 => (on_g, on),
            Some(_) => (off_g, app.theme.faint()),
            None => continue,
        };
        buf[(area.x + i as u16, area.y)]
            .set_char(g)
            .set_style(Style::new().fg(c));
    }
}

/// "12s", "4m", "2h" since an instant.
pub fn ago(t: std::time::Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

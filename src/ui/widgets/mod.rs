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
use ratatui::text::{Line, Span};
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

/// Measurements that change smoothly (interpolated across gaps), as opposed
/// to states and settings (drawn as steps).
pub fn continuous(metric: Metric) -> bool {
    !matches!(metric, Metric::On | Metric::Running | Metric::Mode | Metric::HeatSetpoint | Metric::CoolSetpoint)
}

/// The graph palette for a metric.
pub fn palette(metric: Metric) -> crate::ui::theme::Palette {
    use crate::ui::theme::{BATTERY, HEATING, HUMIDITY, POWER, TEMPERATURE};
    match metric {
        Metric::Humidity => HUMIDITY,
        Metric::On => POWER,
        Metric::Running | Metric::HeatDemand => HEATING,
        Metric::Battery => BATTERY,
        _ => TEMPERATURE,
    }
}

/// Samples for `metric` (and an overlay metric) across `width` cells.
fn graph_samples(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(Samples, Samples)> {
    let d = &app.devices[device];
    let h = d.history.get(&metric)?;
    let now = crate::state::unix_minute();
    let n = Graph::samples_for(width, app.theme.ascii);
    let samples = series::sample(h, now, app.range_minutes(), n, continuous(metric));
    // Setpoints change in steps, so they're never interpolated.
    let over = overlay
        .filter(|_| d.current.get(&Metric::Mode).copied().flatten() != Some(0.0))
        .and_then(|m| d.history.get(&m))
        .map(|h| series::sample(h, now, app.range_minutes(), n, false))
        .unwrap_or_default();
    Some((samples, over))
}

/// The value range a graph of `metric` uses at this width.
pub fn metric_range(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(f64, f64)> {
    let (samples, over) = graph_samples(app, device, metric, overlay, width)?;
    if matches!(metric, Metric::On | Metric::Running) {
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
    let p = palette(metric);
    let colour = |frac: f64| t.graph(p, frac);
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

/// A tile graph: the bare graph plus scale labels at the top and bottom
/// left, as btop's net graph does.
pub fn metric_graph(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, area: Rect, buf: &mut Buffer) {
    if !metric_graph_bare(app, device, metric, overlay, area, buf) {
        return;
    }
    // Scale labels; on/off graphs speak for themselves.
    if area.height >= 3
        && area.width >= 20
        && !matches!(metric, Metric::On | Metric::Running)
        && let Some((lo, hi)) = metric_range(app, device, metric, overlay, area.width)
    {
        let fmt = |v: f64| match metric {
            Metric::Temperature => fmt_temp(app, v),
            _ => format!("{v:.0}%"),
        };
        let style = Style::new().fg(app.theme.dim());
        Line::styled(fmt(hi), style).render(Rect::new(area.x, area.y, area.width, 1), buf);
        Line::styled(fmt(lo), style).render(Rect::new(area.x, area.bottom() - 1, area.width, 1), buf);
    }
}

/// A btop box: square corners and titles cut into the border as
/// `┌─┐title┌───┐status┌─┐`. Returns the inner area.
pub fn btop_box(
    app: &App,
    area: Rect,
    buf: &mut Buffer,
    colour: Color,
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
) -> Rect {
    use ratatui::widgets::{Block, BorderType};
    let edge = Style::new().fg(colour);
    let wrap = |spans: Vec<Span<'static>>| {
        let mut v = vec![Span::styled("─┐", edge)];
        v.extend(spans);
        v.push(Span::styled("┌", edge));
        Line::from(v)
    };
    let mut block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(edge);
    let inner = block.inner(area);
    let has_text = |spans: &[Span]| spans.iter().any(|s| !s.content.trim().is_empty());
    let left = has_text(&left).then(|| wrap(left));
    let right = has_text(&right).then(|| {
        let mut l = wrap(right);
        l.spans.push(Span::styled("─", edge));
        l
    });
    // Titles only if they fit together; the left one wins.
    let lw = left.as_ref().map_or(0, |l| l.width() as u16);
    let rw = right.as_ref().map_or(0, |l| l.width() as u16);
    if let Some(l) = left {
        block = block.title(l);
    }
    if let Some(r) = right
        && lw + rw + 3 <= area.width
    {
        block = block.title(r.alignment(Alignment::Right));
    }
    let _ = app;
    block.render(area, buf);
    inner
}

/// A btop row: bold label, a dark `─` leader, and the value right-aligned.
pub fn label_row(app: &App, area: Rect, buf: &mut Buffer, label: &str, value: Line<'static>) {
    let t = &app.theme;
    if area.is_empty() {
        return;
    }
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_char('─').set_style(Style::new().fg(t.faint()));
    }
    let label = Line::from(vec![Span::styled(format!("{label}:"), t.bold(t.title())), Span::raw(" ")]);
    let lw = label.width() as u16;
    label.render(area, buf);
    let mut value = value;
    value.spans.insert(0, Span::raw(" "));
    if (value.width() as u16) + lw <= area.width {
        value.alignment(Alignment::Right).render(area, buf);
    }
}

/// A btop row with a meter between the label and the value:
/// `Humidity: ■■■■■■■■■■■■····· 54%`. Returns the meter's area.
pub fn meter_row(
    app: &App,
    area: Rect,
    buf: &mut Buffer,
    label: &str,
    frac: f64,
    colour: &dyn Fn(f64) -> Color,
    value: Line<'static>,
) -> Rect {
    let t = &app.theme;
    if area.is_empty() {
        return Rect::default();
    }
    let label = Line::from(vec![Span::styled(format!("{label}:"), t.bold(t.title())), Span::raw(" ")]);
    let lw = label.width() as u16;
    label.render(area, buf);
    let mut value = value;
    value.spans.insert(0, Span::raw(" "));
    let vw = value.width() as u16;
    value.alignment(Alignment::Right).render(area, buf);
    let meter = Rect::new(area.x + lw, area.y, area.width.saturating_sub(lw + vw), 1);
    graph::meter(meter, buf, frac, colour, t.faint(), t.ascii);
    meter
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

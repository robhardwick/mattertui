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

/// Whether a device is heating now: running state or demand says so, and
/// it isn't switched off.
pub fn is_heating(d: &DeviceState) -> bool {
    let on = |m| current(d, m).is_some_and(|v| v != 0.0);
    current(d, Metric::Mode) != Some(0.0)
        && (current(d, Metric::Running).is_some_and(|r| r as u64 & 1 != 0) || on(Metric::HeatDemand))
}

/// Whether a leak sensor currently detects water.
pub fn is_leaking(d: &DeviceState) -> bool {
    current(d, Metric::Leak).is_some_and(|v| v != 0.0)
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
    !(metric.binary() || matches!(metric, Metric::Mode | Metric::HeatSetpoint | Metric::CoolSetpoint))
}

/// A room's hue: its configured `accent`, else the next in the cycle.
pub fn room_hue(app: &App, room: usize) -> usize {
    let cfg = &app.config.rooms[room];
    app.theme.hue(cfg.accent.as_ref().map(|a| a.get_ref().as_str()), room)
}

/// A device's hue. A featured device shares its room's (its graph *is* the
/// room); others take the hues after the room's, in order, so neighbouring
/// graphs differ. A device's own `accent` wins.
pub fn device_hue(app: &App, device: usize) -> usize {
    let cfg = &app.config.devices[device];
    let Some(room) = app.rooms.iter().position(|r| r.contains(&device)) else {
        return 0;
    };
    let rh = room_hue(app, room);
    if let Some(a) = &cfg.accent {
        return app.theme.hue(Some(a.get_ref()), rh);
    }
    if cfg.featured {
        return rh;
    }
    let k = app.rooms[room]
        .iter()
        .filter(|&&d| !app.config.devices[d].featured)
        .position(|&d| d == device)
        .unwrap_or(0);
    (rh + 1 + k) % crate::ui::theme::HUES.len()
}

/// Shortest span a graph is stretched over, so a few readings aren't
/// smeared across the whole width.
const MIN_SPAN_MINUTES: i64 = 5;

/// Minutes a graph of `h` covers: the selected range, or less while history
/// is shorter, so the data starts at the left edge and grows rightwards
/// until it fills the range, then scrolls.
pub fn span(app: &App, h: &History) -> i64 {
    let now = crate::state::unix_minute();
    let have = h.buckets().next().map_or(0, |b| now - b.minute);
    have.clamp(MIN_SPAN_MINUTES, app.range_minutes().max(MIN_SPAN_MINUTES))
}

/// "12m", "1h", "1h30m", "2d".
pub fn fmt_span(minutes: i64) -> String {
    match minutes {
        ..60 => format!("{minutes}m"),
        _ if minutes % 1440 == 0 => format!("{}d", minutes / 1440),
        _ if minutes % 60 == 0 => format!("{}h", minutes / 60),
        _ if minutes < 1440 => format!("{}h{:02}m", minutes / 60, minutes % 60),
        _ => format!("{}d{}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

/// Samples for `metric` (and an overlay metric) across `width` cells.
fn graph_samples(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(Samples, Samples)> {
    let d = &app.devices[device];
    let h = d.history.get(&metric)?;
    let now = crate::state::unix_minute();
    let n = Graph::samples_for(width, app.theme.ascii);
    let span = span(app, h);
    let samples = series::sample(h, now, span, n, continuous(metric));
    // Setpoints change in steps, so they're never interpolated.
    let over = overlay
        .filter(|_| d.current.get(&Metric::Mode).copied().flatten() != Some(0.0))
        .and_then(|m| d.history.get(&m))
        .map(|h| series::sample(h, now, span, n, false))
        .unwrap_or_default();
    Some((samples, over))
}

/// The value range a graph of `metric` uses at this width.
pub fn metric_range(app: &App, device: usize, metric: Metric, overlay: Option<Metric>, width: u16) -> Option<(f64, f64)> {
    let (samples, over) = graph_samples(app, device, metric, overlay, width)?;
    if metric.binary() {
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
    if metric.binary() {
        // A faint baseline, so "off" or "dry" reads as a line, not a blank.
        let y = area.bottom() - 1;
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(if t.ascii { '.' } else { '⣀' }).set_style(Style::new().fg(t.faint()));
        }
    }
    let h = device_hue(app, device);
    let colour = |frac: f64| t.graph(h, frac);
    Graph {
        samples: &samples,
        lo,
        hi,
        colour: &colour,
        overlay: (!over.is_empty()).then_some((over.as_slice(), t.overlay())),
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
        && !metric.binary()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::unix_minute;

    #[test]
    fn graphs_fit_short_history_then_scroll() {
        let mut app = crate::ui::tests_support::fake_app(); // range 1h
        let now = unix_minute();
        let mut h = History::new(24 * 60);
        h.record(now - 11, 20.0);
        assert_eq!(span(&app, &h), 11, "11 minutes of data fill the width");
        h.record(now, 21.0);
        let mut short = History::new(24 * 60);
        short.record(now, 1.0);
        assert_eq!(span(&app, &short), MIN_SPAN_MINUTES, "never narrower than the floor");
        let mut long = History::new(24 * 60);
        long.record(now - 600, 1.0);
        assert_eq!(span(&app, &long), 60, "capped at the selected range");
        app.view = crate::ui::app::View::Dashboard;
        // The leftmost column has data, and short history still ramps
        // smoothly rather than in whole-minute steps.
        let samples = series::sample(&h, now, span(&app, &h), 40, true);
        assert!(samples[0].is_some(), "{samples:?}");
        let distinct: std::collections::BTreeSet<_> = samples.iter().flatten().map(|v| (v * 1000.0) as i64).collect();
        assert!(distinct.len() > 30, "{} distinct of 40", distinct.len());
    }

    #[test]
    fn spans_format_compactly() {
        assert_eq!(fmt_span(12), "12m");
        assert_eq!(fmt_span(60), "1h");
        assert_eq!(fmt_span(90), "1h30m");
        assert_eq!(fmt_span(1440), "1d");
    }
}

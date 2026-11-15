//! Rendering. Every function draws into a `Buffer` so the dashboard can be
//! rendered off-screen and scrolled.

pub mod chrome;
pub mod detail;
pub mod room;
pub mod tile;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use super::app::{App, Health};
use super::series;
use crate::config::TemperatureUnit;
use crate::state::{DeviceState, History, Metric};

/// "21.5°" in the configured unit.
pub fn fmt_temp(app: &App, celsius: f64) -> String {
    match app.config.ui.temperature_unit {
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

pub fn health_badge(app: &App, h: Health) -> Option<(&'static str, Color)> {
    let t = &app.theme;
    Some(match h {
        Health::Live => return None,
        Health::Unpaired => ("not paired", t.dim()),
        Health::Connecting => ("connecting…", t.dim()),
        Health::Stale => ("stale", t.warn()),
        Health::Reconnecting => ("reconnecting…", t.warn()),
        Health::Offline => ("offline", t.bad()),
    })
}

/// A one-row sparkline of `metric` over the selected range, each column
/// coloured by `colour`.
pub fn sparkline(
    app: &App,
    d: &DeviceState,
    metric: Metric,
    area: Rect,
    buf: &mut Buffer,
    colour: impl Fn(f64) -> Color,
) {
    let Some(h) = d.history.get(&metric) else {
        return;
    };
    let now = crate::state::unix_minute();
    let values = series::sample(h, now, app.range_minutes(), area.width as usize);
    let (lo, hi) = values
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let glyphs = if app.theme.ascii {
        series::SPARK_ASCII
    } else {
        series::SPARK
    };
    for (i, v) in values.iter().enumerate() {
        if let Some(v) = v {
            let g = glyphs[series::level(*v, lo, hi)];
            buf[(area.x + i as u16, area.y)]
                .set_char(g)
                .set_style(Style::new().fg(colour(*v)));
        }
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

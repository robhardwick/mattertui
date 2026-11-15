//! Named icons: emoji, with ASCII on the bare Linux console.
//!
//! Only emoji whose *default* presentation is emoji are used. Those are two
//! cells wide in ratatui, tmux and terminals alike, so layouts stay aligned
//! over SSH + tmux. Text-default symbols (☀ ❄ 🛋 🛏) and anything needing a
//! variation selector are avoided: tmux and terminals disagree on their
//! width, which corrupts the screen. The one exception is 🌡 (there is no
//! emoji-default thermometer); [`cell`] pads it to two cells.

use ratatui::text::Span;

use crate::config::DeviceKind;

/// (name, emoji, ascii)
const ICONS: &[(&str, &str, &str)] = &[
    // Devices
    ("lamp", "💡", "*"),
    ("bulb", "💡", "*"),
    ("plug", "🔌", "o"),
    ("fan", "🌀", "x"),
    ("tv", "📺", "="),
    ("thermostat", "🔥", "~"),
    ("thermometer", "🌡", "t"),
    // Readings and states
    ("humidity", "💧", "%"),
    ("battery", "🔋", "#"),
    ("fire", "🔥", "~"),
    ("cool", "🧊", "*"),
    // Rooms
    ("home", "🏠", "^"),
    ("sofa", "🏠", "^"),
    ("bed", "🛌", ")"),
    ("moon", "🌙", ")"),
    ("sun", "🌞", "*"),
    ("kitchen", "🍳", "~"),
    ("bath", "🛁", "~"),
    ("office", "💻", "#"),
    ("desk", "💻", "#"),
    ("garden", "🌳", "&"),
    ("tree", "🌳", "&"),
    ("teddy", "🧸", "@"),
    ("games", "🎮", "@"),
    ("books", "📚", "="),
    ("garage", "🚗", "="),
    ("star", "⭐", "*"),
    ("heart", "💛", "<3"),
    ("snow", "⛄", "*"),
];

/// Resolve a configured icon: a known name, or a literal glyph (1–2 chars).
pub fn resolve(icon: &str, ascii: bool) -> Option<&str> {
    if let Some((_, u, a)) = ICONS.iter().find(|(n, _, _)| *n == icon) {
        return Some(if ascii { a } else { u });
    }
    is_literal(icon).then_some(icon)
}

pub fn is_valid(icon: &str) -> bool {
    ICONS.iter().any(|(n, _, _)| *n == icon) || is_literal(icon)
}

fn is_literal(icon: &str) -> bool {
    let n = icon.chars().count();
    (1..=2).contains(&n) && !icon.chars().all(|c| c.is_ascii_alphabetic())
}

/// A built-in icon by name (programming error if missing).
pub fn get(name: &str, ascii: bool) -> &'static str {
    ICONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map_or("?", |(_, e, a)| if ascii { a } else { e })
}

/// An icon padded to two cells, so emoji and narrow fallbacks line up.
pub fn cell(name: &str, ascii: bool) -> String {
    pad2(get(name, ascii))
}

/// Pad a glyph to two cells.
pub fn pad2(glyph: &str) -> String {
    if Span::raw(glyph).width() < 2 {
        format!("{glyph} ")
    } else {
        glyph.to_string()
    }
}

pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _, _)| *n)
}

pub fn default_for(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Plug => "lamp",
        DeviceKind::Thermostat => "thermostat",
        DeviceKind::TemperatureSensor => "thermometer",
    }
}

/// The glyph to draw for a device or room, padded to two cells.
pub fn glyph(icon: Option<&str>, fallback: &str, ascii: bool) -> String {
    pad2(icon.and_then(|i| resolve(i, ascii)).or_else(|| resolve(fallback, ascii)).unwrap_or("·"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_literals_and_ascii() {
        assert_eq!(resolve("lamp", false), Some("💡"));
        assert_eq!(resolve("lamp", true), Some("*"));
        assert_eq!(resolve("☂", false), Some("☂"));
        assert_eq!(resolve("lampp", false), None);
        assert!(!is_valid("ab"));
        assert!(is_valid("★"));
    }

    /// Every emoji must be two cells wide everywhere (emoji-default
    /// presentation, no variation selectors), except the padded thermometer.
    #[test]
    fn emoji_are_double_width() {
        for (name, emoji, ascii) in ICONS {
            assert!(!emoji.contains('\u{FE0F}'), "{name}: no variation selectors");
            let w = Span::raw(*emoji).width();
            if *name == "thermometer" {
                assert_eq!(w, 1, "thermometer is text-default");
            } else {
                assert_eq!(w, 2, "{name} ({emoji}) must be emoji-default");
            }
            assert!(cell(name, false).starts_with(emoji));
            assert!(ascii.is_ascii());
        }
        assert_eq!(Span::raw(cell("thermometer", false)).width(), 2);
        assert_eq!(Span::raw(cell("lamp", true)).width(), 2);
    }
}


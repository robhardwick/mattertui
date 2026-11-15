//! Named icons. Plain Unicode symbols (not emoji or nerdfont) so widths are
//! stable in tmux and no special font is needed; ASCII on the bare console.

use crate::config::DeviceKind;

/// (name, unicode, ascii)
const ICONS: &[(&str, &str, &str)] = &[
    ("lamp", "☼", "*"),
    ("bulb", "☼", "*"),
    ("plug", "◉", "o"),
    ("fan", "✻", "x"),
    ("tv", "▭", "="),
    ("thermostat", "♨", "~"),
    ("thermometer", "°", "'"),
    ("humidity", "≈", "%"),
    ("battery", "▮", "#"),
    ("sofa", "⌂", "^"),
    ("home", "⌂", "^"),
    ("bed", "☾", ")"),
    ("moon", "☾", ")"),
    ("sun", "☀", "*"),
    ("kitchen", "♨", "~"),
    ("bath", "≈", "~"),
    ("office", "▣", "#"),
    ("desk", "▣", "#"),
    ("garden", "♣", "&"),
    ("tree", "♣", "&"),
    ("star", "★", "*"),
    ("heart", "♥", "<3"),
    ("snow", "❄", "*"),
    ("fire", "♨", "~"),
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

/// The glyph to draw for a device or room.
pub fn glyph(icon: Option<&str>, fallback: &str, ascii: bool) -> String {
    icon.and_then(|i| resolve(i, ascii))
        .or_else(|| resolve(fallback, ascii))
        .unwrap_or("·")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_literals_and_ascii() {
        assert_eq!(resolve("lamp", false), Some("☼"));
        assert_eq!(resolve("lamp", true), Some("*"));
        assert_eq!(resolve("☂", false), Some("☂"));
        assert_eq!(resolve("lampp", false), None);
        assert!(!is_valid("ab"));
        assert!(is_valid("★"));
    }
}

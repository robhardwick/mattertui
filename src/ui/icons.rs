//! A few icons, used sparingly. Every glyph is a plain text symbol that has
//! **no emoji presentation at all** (Unicode `Emoji=No`), so no terminal or
//! font can ever render it as a colour emoji, with or without variation
//! selectors. All are one cell wide and common in monospace fonts. The bare
//! Linux console gets ASCII.

use crate::config::DeviceKind;

/// (name, symbol, ascii)
const ICONS: &[(&str, &str, &str)] = &[
    // Devices
    ("lamp", "☼", "*"),
    ("bulb", "☼", "*"),
    ("plug", "◉", "o"),
    ("fan", "✻", "x"),
    ("tv", "▭", "="),
    ("thermostat", "≋", "~"),
    ("thermometer", "°", "'"),
    // Readings and states
    ("humidity", "≈", "%"),
    ("battery", "▮", "#"),
    ("heating", "≋", "~"),
    ("cool", "✻", "*"),
    // Rooms
    ("home", "⌂", "^"),
    ("sofa", "⌂", "^"),
    ("bed", "☾", ")"),
    ("moon", "☾", ")"),
    ("sun", "☼", "*"),
    ("kitchen", "◎", "@"),
    ("bath", "≈", "~"),
    ("office", "▣", "#"),
    ("desk", "▣", "#"),
    ("garden", "↟", "^"),
    ("tree", "↟", "^"),
    ("teddy", "ʘ", "@"),
    ("games", "◈", "+"),
    ("books", "▤", "="),
    ("garage", "▥", "="),
    ("star", "★", "*"),
    ("heart", "♡", "<"),
    ("snow", "✻", "*"),
];

/// Resolve a configured icon name.
pub fn resolve(icon: &str, ascii: bool) -> Option<&'static str> {
    ICONS
        .iter()
        .find(|(n, _, _)| *n == icon)
        .map(|(_, s, a)| if ascii { *a } else { *s })
}

pub fn is_valid(icon: &str) -> bool {
    ICONS.iter().any(|(n, _, _)| *n == icon)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, _, _)| *n)
}

/// A built-in icon by name ("?" if missing: a programming error).
pub fn get(name: &str, ascii: bool) -> &'static str {
    resolve(name, ascii).unwrap_or("?")
}

pub fn default_for(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Plug => "lamp",
        DeviceKind::Thermostat => "thermostat",
        DeviceKind::TemperatureSensor => "thermometer",
    }
}

/// The glyph for a device or room: its configured icon, else `fallback`.
pub fn glyph(icon: Option<&str>, fallback: Option<&str>, ascii: bool) -> Option<&'static str> {
    icon.and_then(|i| resolve(i, ascii)).or_else(|| fallback.and_then(|f| resolve(f, ascii)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every BMP code point with `Emoji=Yes` (Unicode emoji-data.txt). Any of
    /// these can render as a colour emoji in some terminal, so none may be
    /// an icon. (Supplementary-plane emoji are excluded by the range check.)
    const BMP_EMOJI: &[(u32, u32)] = &[
        (0x00A9, 0x00A9), (0x00AE, 0x00AE), (0x203C, 0x203C), (0x2049, 0x2049),
        (0x2122, 0x2122), (0x2139, 0x2139), (0x2194, 0x2199), (0x21A9, 0x21AA),
        (0x231A, 0x231B), (0x2328, 0x2328), (0x23CF, 0x23CF), (0x23E9, 0x23F3),
        (0x23F8, 0x23FA), (0x24C2, 0x24C2), (0x25AA, 0x25AB), (0x25B6, 0x25B6),
        (0x25C0, 0x25C0), (0x25FB, 0x25FE), (0x2600, 0x2604), (0x260E, 0x260E),
        (0x2611, 0x2611), (0x2614, 0x2615), (0x2618, 0x2618), (0x261D, 0x261D),
        (0x2620, 0x2620), (0x2622, 0x2623), (0x2626, 0x2626), (0x262A, 0x262A),
        (0x262E, 0x262F), (0x2638, 0x263A), (0x2640, 0x2640), (0x2642, 0x2642),
        (0x2648, 0x2653), (0x265F, 0x2660), (0x2663, 0x2663), (0x2665, 0x2666),
        (0x2668, 0x2668), (0x267B, 0x267B), (0x267E, 0x267F), (0x2692, 0x2697),
        (0x2699, 0x2699), (0x269B, 0x269C), (0x26A0, 0x26A1), (0x26A7, 0x26A7),
        (0x26AA, 0x26AB), (0x26B0, 0x26B1), (0x26BD, 0x26BE), (0x26C4, 0x26C5),
        (0x26C8, 0x26C8), (0x26CE, 0x26CF), (0x26D1, 0x26D1), (0x26D3, 0x26D4),
        (0x26E9, 0x26EA), (0x26F0, 0x26F5), (0x26F7, 0x26FA), (0x26FD, 0x26FD),
        (0x2702, 0x2702), (0x2705, 0x2705), (0x2708, 0x270D), (0x270F, 0x270F),
        (0x2712, 0x2712), (0x2714, 0x2714), (0x2716, 0x2716), (0x271D, 0x271D),
        (0x2721, 0x2721), (0x2728, 0x2728), (0x2733, 0x2734), (0x2744, 0x2744),
        (0x2747, 0x2747), (0x274C, 0x274C), (0x274E, 0x274E), (0x2753, 0x2755),
        (0x2757, 0x2757), (0x2763, 0x2764), (0x2795, 0x2797), (0x27A1, 0x27A1),
        (0x27B0, 0x27B0), (0x27BF, 0x27BF), (0x2934, 0x2935), (0x2B05, 0x2B07),
        (0x2B1B, 0x2B1C), (0x2B50, 0x2B50), (0x2B55, 0x2B55), (0x3030, 0x3030),
        (0x303D, 0x303D), (0x3297, 0x3297), (0x3299, 0x3299),
    ];

    fn can_be_emoji(c: char) -> bool {
        let c = c as u32;
        c >= 0x1F000 || BMP_EMOJI.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
    }

    #[test]
    fn icons_can_never_render_as_emoji() {
        for (name, symbol, ascii) in ICONS {
            for c in symbol.chars() {
                assert!(!can_be_emoji(c), "{name}: {c:?} (U+{:04X}) has an emoji presentation", c as u32);
                assert!(c != '\u{FE0E}' && c != '\u{FE0F}', "{name}: no variation selectors");
            }
            assert_eq!(ratatui::text::Span::raw(*symbol).width(), 1, "{name} is one cell");
            assert!(ascii.is_ascii());
        }
    }

    /// Other symbols the UI draws directly must pass the same check.
    #[test]
    fn ui_symbols_can_never_render_as_emoji() {
        for c in "●○◆▲▼↑↓→←■⣿⠀─│┌┐└┘┤├¹²³⁴⁵⁶⁷⁸⁹⁰·…".chars() {
            assert!(!can_be_emoji(c), "{c:?} (U+{:04X})", c as u32);
        }
        // The check itself catches the usual suspects.
        for c in "☀❄♨⚡⭐▶↔".chars() {
            assert!(can_be_emoji(c), "{c:?} should be flagged");
        }
    }

    #[test]
    fn resolves_names() {
        assert_eq!(resolve("lamp", false), Some("☼"));
        assert_eq!(resolve("lamp", true), Some("*"));
        assert_eq!(resolve("nope", false), None);
        assert_eq!(glyph(None, Some("sofa"), false), Some("⌂"));
    }
}

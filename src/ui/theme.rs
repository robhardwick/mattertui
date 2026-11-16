//! Colours, gradients and terminal capability fallback.
//!
//! One scheme everywhere: **white** for titles, labels and values, **teal**
//! (in shades) for structure and secondary text, **orange** for highlights,
//! selection and warmth. Graphs are the exception: subtle two-tone vertical
//! gradients in complementary hues (lavender, blue, amber). Tuned for a black
//! terminal; the app never paints a background.

use ratatui::style::{Color, Modifier, Style};

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// 24-bit colour; otherwise a 16-colour palette.
    pub truecolor: bool,
    /// Bare Linux console: ASCII graphs (no braille glyphs).
    pub ascii: bool,
}

/// Room box colours: shades of teal, assigned in declaration order unless a
/// room names one.
pub const ACCENTS: &[(&str, Rgb, Color)] = &[
    ("teal", (64, 170, 170), Color::Cyan),
    ("aqua", (110, 206, 200), Color::LightCyan),
    ("deep", (42, 132, 140), Color::Cyan),
    ("mint", (120, 200, 176), Color::LightGreen),
    ("steel", (88, 146, 166), Color::Blue),
    ("sea", (72, 162, 148), Color::Green),
    ("pale", (156, 212, 210), Color::White),
];

/// A graph palette: colour at the baseline and at the top. Graphs blend
/// between them by row height, which reads as a subtle gradient.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    low: Rgb,
    high: Rgb,
    fallback: Color,
}

pub const TEMPERATURE: Palette = Palette { low: (58, 66, 140), high: (176, 166, 238), fallback: Color::Magenta };
pub const HUMIDITY: Palette = Palette { low: (34, 88, 132), high: (120, 196, 236), fallback: Color::Blue };
pub const POWER: Palette = Palette { low: (132, 88, 34), high: (250, 196, 116), fallback: Color::Yellow };
pub const HEATING: Palette = Palette { low: (136, 58, 28), high: (255, 146, 84), fallback: Color::LightRed };
pub const BATTERY: Palette = Palette { low: (40, 110, 104), high: (128, 214, 196), fallback: Color::Cyan };

impl Theme {
    pub fn detect() -> Self {
        let term = std::env::var("TERM").unwrap_or_default();
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        Self {
            truecolor: matches!(colorterm.as_str(), "truecolor" | "24bit"),
            ascii: term == "linux",
        }
    }

    fn pick(&self, rgb: Rgb, fallback: Color) -> Color {
        if self.truecolor {
            Color::Rgb(rgb.0, rgb.1, rgb.2)
        } else {
            fallback
        }
    }

    // --- Structure (teal) ---

    /// Plain box frames and rules.
    pub fn frame(&self) -> Color {
        self.pick((40, 100, 108), Color::DarkGray)
    }
    /// Titles, labels and values: white.
    pub fn title(&self) -> Color {
        self.pick((240, 242, 242), Color::White)
    }
    /// Secondary text: teal-grey.
    pub fn dim(&self) -> Color {
        self.pick((112, 156, 160), Color::Gray)
    }
    /// Dividers, leaders and meter tracks.
    pub fn faint(&self) -> Color {
        self.pick((38, 58, 62), Color::DarkGray)
    }

    // --- Data (white, with orange emphasis) ---

    /// Plain values.
    pub fn fg(&self) -> Color {
        self.pick((222, 228, 228), Color::White)
    }
    /// The highlight: hotkeys, room numbers, selection.
    pub fn hi(&self) -> Color {
        self.pick((255, 150, 60), Color::Yellow)
    }
    /// Background of the selected title.
    pub fn selected_bg(&self) -> Color {
        self.pick((122, 60, 18), Color::Yellow)
    }
    /// Warm light for lights that are on.
    pub fn glow(&self) -> Color {
        self.pick((255, 184, 92), Color::Yellow)
    }
    /// Peak of the "breathing" glow.
    pub fn glow_bright(&self) -> Color {
        self.pick((255, 226, 172), Color::LightYellow)
    }
    pub fn heat(&self) -> Color {
        self.pick((255, 122, 52), Color::LightRed)
    }
    pub fn cool(&self) -> Color {
        self.pick((124, 214, 222), Color::LightCyan)
    }

    // --- Status (teal good, orange warn, red-orange bad) ---

    pub fn good(&self) -> Color {
        self.pick((92, 204, 184), Color::Cyan)
    }
    pub fn warn(&self) -> Color {
        self.pick((255, 190, 92), Color::Yellow)
    }
    pub fn bad(&self) -> Color {
        self.pick((255, 96, 64), Color::LightRed)
    }

    // --- Styles ---

    pub fn text(&self) -> Style {
        Style::new().fg(self.fg())
    }
    pub fn muted(&self) -> Style {
        Style::new().fg(self.dim())
    }
    pub fn bold(&self, c: Color) -> Style {
        Style::new().fg(c).add_modifier(Modifier::BOLD)
    }

    /// Tint for the `index`th room, or the named one.
    pub fn accent(&self, name: Option<&str>, index: usize) -> Color {
        let entry = name
            .and_then(|n| ACCENTS.iter().find(|(a, _, _)| *a == n))
            .unwrap_or(&ACCENTS[index % ACCENTS.len()]);
        self.pick(entry.1, entry.2)
    }

    /// A graph palette's colour at a height fraction (0 = baseline).
    pub fn graph(&self, p: Palette, frac: f64) -> Color {
        if !self.truecolor {
            return p.fallback;
        }
        let t = frac.clamp(0.0, 1.0);
        Color::Rgb(lerp(p.low.0, p.high.0, t), lerp(p.low.1, p.high.1, t), lerp(p.low.2, p.high.2, t))
    }
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
}

pub fn accent_names() -> impl Iterator<Item = &'static str> {
    ACCENTS.iter().map(|(n, _, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TC: Theme = Theme {
        truecolor: true,
        ascii: false,
    };

    #[test]
    fn palettes_blend_by_height() {
        assert_eq!(TC.graph(TEMPERATURE, 0.0), Color::Rgb(58, 66, 140));
        assert_eq!(TC.graph(TEMPERATURE, 1.0), Color::Rgb(176, 166, 238));
        assert_eq!(TC.graph(TEMPERATURE, 7.0), Color::Rgb(176, 166, 238));
    }

    #[test]
    fn sixteen_colour_fallback() {
        let t = Theme {
            truecolor: false,
            ascii: false,
        };
        assert_eq!(t.graph(HUMIDITY, 0.5), Color::Blue);
        assert_eq!(t.accent(Some("aqua"), 0), Color::LightCyan);
        assert_eq!(t.hi(), Color::Yellow);
    }
}

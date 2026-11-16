//! Colours, gradients and terminal capability fallback.
//!
//! One palette, tuned for a black terminal background: teal and blue for
//! structure (frames, titles, labels, graph lows), orange and amber for data
//! and emphasis (highlights, graph highs, lights on, heating). Red is only for
//! errors and hot temperatures. The app never paints a background.

use ratatui::style::{Color, Modifier, Style};

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// 24-bit colour; otherwise a 16-colour palette.
    pub truecolor: bool,
    /// Bare Linux console: ASCII icons and charts (no braille glyphs).
    pub ascii: bool,
}

/// Room tints: subtle variations within the teal/blue family, assigned in
/// declaration order unless a room names one.
pub const ACCENTS: &[(&str, Rgb, Color)] = &[
    ("teal", (72, 178, 178), Color::Cyan),
    ("steel", (96, 146, 196), Color::Blue),
    ("sea", (78, 182, 146), Color::Green),
    ("cyan", (86, 192, 222), Color::LightCyan),
    ("slate", (122, 156, 178), Color::Gray),
    ("jade", (64, 160, 124), Color::LightGreen),
    ("aqua", (110, 206, 200), Color::Cyan),
];

/// Temperature: cold blue → teal → seafoam → amber → hot red-orange.
const TEMP_STOPS: &[(f64, Rgb, Color)] = &[
    (14.0, (76, 128, 220), Color::Blue),
    (17.0, (64, 186, 196), Color::Cyan),
    (20.0, (146, 206, 170), Color::Green),
    (23.0, (255, 176, 72), Color::Yellow),
    (26.0, (255, 92, 64), Color::Red),
];

const HUMIDITY_STOPS: &[(f64, Rgb, Color)] = &[
    (30.0, (112, 150, 166), Color::Gray),
    (50.0, (84, 176, 210), Color::Cyan),
    (70.0, (64, 128, 236), Color::Blue),
];

/// Generic graph gradient by height: teal near the baseline rising to orange.
const GRAPH_STOPS: &[(f64, Rgb, Color)] = &[
    (0.0, (32, 96, 112), Color::Blue),
    (0.45, (64, 176, 172), Color::Cyan),
    (0.8, (246, 168, 76), Color::Yellow),
    (1.0, (255, 112, 52), Color::LightRed),
];

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

    // --- Structure (teal family) ---

    /// Box frames and rules.
    pub fn frame(&self) -> Color {
        self.pick((44, 84, 96), Color::DarkGray)
    }
    /// Titles in frames.
    pub fn title(&self) -> Color {
        self.pick((132, 214, 214), Color::Cyan)
    }
    /// Labels and secondary text.
    pub fn dim(&self) -> Color {
        self.pick((112, 140, 150), Color::Gray)
    }
    /// Tracks, leaders and the faintest detail.
    pub fn faint(&self) -> Color {
        self.pick((44, 62, 70), Color::DarkGray)
    }

    // --- Data (white values, orange emphasis) ---

    /// Plain values.
    pub fn fg(&self) -> Color {
        self.pick((228, 232, 234), Color::White)
    }
    /// The highlight colour: selection, numbers to notice.
    pub fn orange(&self) -> Color {
        self.pick((255, 152, 56), Color::Yellow)
    }
    /// Warm light for lights that are on.
    pub fn glow(&self) -> Color {
        self.pick((255, 176, 72), Color::Yellow)
    }
    /// Peak of the "breathing" glow.
    pub fn glow_bright(&self) -> Color {
        self.pick((255, 226, 166), Color::LightYellow)
    }
    pub fn heat(&self) -> Color {
        self.pick((255, 116, 52), Color::LightRed)
    }
    pub fn cool(&self) -> Color {
        self.pick((92, 168, 240), Color::LightBlue)
    }

    // --- Status ---

    pub fn good(&self) -> Color {
        self.pick((84, 204, 164), Color::Green)
    }
    pub fn warn(&self) -> Color {
        self.pick((255, 192, 72), Color::Yellow)
    }
    pub fn bad(&self) -> Color {
        self.pick((255, 88, 88), Color::Red)
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

    pub fn temperature(&self, celsius: f64) -> Color {
        self.gradient(TEMP_STOPS, celsius)
    }

    pub fn humidity(&self, percent: f64) -> Color {
        self.gradient(HUMIDITY_STOPS, percent)
    }

    /// Graph colour at a height fraction (0 = baseline, 1 = top).
    pub fn graph(&self, frac: f64) -> Color {
        self.gradient(GRAPH_STOPS, frac)
    }

    fn gradient(&self, stops: &[(f64, Rgb, Color)], x: f64) -> Color {
        let first = stops[0];
        let last = stops[stops.len() - 1];
        if x <= first.0 {
            return self.pick(first.1, first.2);
        }
        if x >= last.0 {
            return self.pick(last.1, last.2);
        }
        for w in stops.windows(2) {
            let (a, b) = (w[0], w[1]);
            if x <= b.0 {
                let t = (x - a.0) / (b.0 - a.0);
                if !self.truecolor {
                    return if t < 0.5 { a.2 } else { b.2 };
                }
                return Color::Rgb(lerp(a.1.0, b.1.0, t), lerp(a.1.1, b.1.1, t), lerp(a.1.2, b.1.2, t));
            }
        }
        self.pick(last.1, last.2)
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
    fn gradient_clamps_and_interpolates() {
        assert_eq!(TC.temperature(0.0), Color::Rgb(76, 128, 220));
        assert_eq!(TC.temperature(40.0), Color::Rgb(255, 92, 64));
        assert_eq!(TC.temperature(20.0), Color::Rgb(146, 206, 170));
        assert_eq!(TC.graph(0.0), Color::Rgb(32, 96, 112));
        assert_eq!(TC.graph(1.0), Color::Rgb(255, 112, 52));
    }

    #[test]
    fn sixteen_colour_fallback_snaps_to_nearest_stop() {
        let t = Theme {
            truecolor: false,
            ascii: false,
        };
        assert_eq!(t.temperature(20.4), Color::Green);
        assert_eq!(t.temperature(22.8), Color::Yellow);
        assert_eq!(t.accent(Some("steel"), 0), Color::Blue);
        assert_eq!(t.frame(), Color::DarkGray);
    }
}

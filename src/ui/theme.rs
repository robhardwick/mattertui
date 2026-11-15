//! Colours, gradients and terminal capability fallback.

use ratatui::style::{Color, Modifier, Style};

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// 24-bit colour; otherwise a 16-colour palette.
    pub truecolor: bool,
    /// Bare Linux console: ASCII icons and block charts (no braille glyphs).
    pub ascii: bool,
}

/// Room accents, assigned in declaration order unless a room names one.
pub const ACCENTS: &[(&str, Rgb, Color)] = &[
    ("amber", (255, 183, 77), Color::Yellow),
    ("teal", (77, 208, 195), Color::Cyan),
    ("violet", (167, 139, 250), Color::Magenta),
    ("rose", (244, 114, 182), Color::LightMagenta),
    ("sky", (96, 165, 250), Color::LightBlue),
    ("lime", (163, 230, 53), Color::LightGreen),
    ("coral", (255, 138, 101), Color::LightRed),
];

/// Temperature gradient stops (°C → colour).
const TEMP_STOPS: &[(f64, Rgb, Color)] = &[
    (14.0, (59, 130, 246), Color::Blue),
    (17.0, (45, 212, 191), Color::Cyan),
    (20.0, (74, 222, 128), Color::Green),
    (23.0, (251, 191, 36), Color::Yellow),
    (26.0, (239, 68, 68), Color::Red),
];

const HUMIDITY_STOPS: &[(f64, Rgb, Color)] = &[
    (30.0, (148, 163, 184), Color::Gray),
    (50.0, (125, 211, 252), Color::Cyan),
    (70.0, (56, 130, 248), Color::Blue),
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

    pub fn fg(&self) -> Color {
        self.pick((222, 226, 232), Color::White)
    }
    pub fn dim(&self) -> Color {
        self.pick((124, 133, 146), Color::Gray)
    }
    pub fn faint(&self) -> Color {
        self.pick((66, 73, 85), Color::DarkGray)
    }
    pub fn border(&self) -> Color {
        self.pick((58, 64, 76), Color::DarkGray)
    }
    pub fn good(&self) -> Color {
        self.pick((74, 222, 128), Color::Green)
    }
    pub fn warn(&self) -> Color {
        self.pick((251, 191, 36), Color::Yellow)
    }
    pub fn bad(&self) -> Color {
        self.pick((248, 113, 113), Color::Red)
    }
    /// Warm glow for lights that are on.
    pub fn glow(&self) -> Color {
        self.pick((255, 200, 87), Color::Yellow)
    }
    /// Peak of the "breathing" glow for lights that are on.
    pub fn glow_bright(&self) -> Color {
        self.pick((255, 241, 196), Color::LightYellow)
    }
    pub fn heat(&self) -> Color {
        self.pick((255, 112, 67), Color::LightRed)
    }
    pub fn cool(&self) -> Color {
        self.pick((96, 165, 250), Color::LightBlue)
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.fg())
    }
    pub fn muted(&self) -> Style {
        Style::new().fg(self.dim())
    }
    pub fn bold(&self, c: Color) -> Style {
        Style::new().fg(c).add_modifier(Modifier::BOLD)
    }

    /// Accent for the `index`th room, or the named one.
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
        assert_eq!(TC.temperature(0.0), Color::Rgb(59, 130, 246));
        assert_eq!(TC.temperature(40.0), Color::Rgb(239, 68, 68));
        assert_eq!(TC.temperature(20.0), Color::Rgb(74, 222, 128));
        // Halfway between green (20) and amber (23).
        assert_eq!(TC.temperature(21.5), Color::Rgb(163, 207, 82));
    }

    #[test]
    fn sixteen_colour_fallback_snaps_to_nearest_stop() {
        let t = Theme {
            truecolor: false,
            ascii: false,
        };
        assert_eq!(t.temperature(20.4), Color::Green);
        assert_eq!(t.temperature(22.8), Color::Yellow);
        assert_eq!(t.accent(Some("teal"), 0), Color::Cyan);
    }
}

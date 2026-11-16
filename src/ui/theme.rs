//! Colours, gradients and terminal capability fallback.
//!
//! Modelled on btop's default theme for a black terminal: each room box has
//! its own colour (green, orange, magenta, red, …), labels and values are
//! bold white, dividers and meter tracks are dark grey, and graphs and meters
//! run green → yellow → red. The app never paints a background.

use ratatui::style::{Color, Modifier, Style};

type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// 24-bit colour; otherwise a 16-colour palette.
    pub truecolor: bool,
    /// Bare Linux console: ASCII graphs (no braille glyphs).
    pub ascii: bool,
}

/// Room box colours, assigned in declaration order unless a room names one.
pub const ACCENTS: &[(&str, Rgb, Color)] = &[
    ("green", (84, 178, 96), Color::Green),
    ("orange", (204, 132, 52), Color::Yellow),
    ("magenta", (186, 72, 186), Color::Magenta),
    ("red", (196, 70, 70), Color::Red),
    ("blue", (92, 100, 214), Color::Blue),
    ("cyan", (52, 160, 170), Color::Cyan),
    ("olive", (160, 156, 52), Color::LightYellow),
];

/// Temperature: cold blue → teal → green → yellow → hot red.
const TEMP_STOPS: &[(f64, Rgb, Color)] = &[
    (14.0, (72, 151, 212), Color::Blue),
    (17.0, (64, 200, 190), Color::Cyan),
    (20.0, (80, 240, 149), Color::Green),
    (23.0, (242, 226, 102), Color::Yellow),
    (27.0, (250, 30, 30), Color::Red),
];

/// Humidity: btop's "cached" blues.
const HUMIDITY_STOPS: &[(f64, Rgb, Color)] = &[
    (30.0, (40, 110, 160), Color::Blue),
    (50.0, (116, 230, 252), Color::Cyan),
    (70.0, (38, 197, 255), Color::LightBlue),
];

/// btop's cpu gradient, by height: green → yellow → red.
const GRAPH_STOPS: &[(f64, Rgb, Color)] = &[
    (0.0, (80, 240, 149), Color::Green),
    (0.5, (242, 226, 102), Color::Yellow),
    (1.0, (250, 30, 30), Color::Red),
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

    // --- Structure ---

    /// Plain box frames (log, detail fallbacks) and rules.
    pub fn frame(&self) -> Color {
        self.pick((88, 88, 88), Color::DarkGray)
    }
    /// Titles and labels: btop's bright white.
    pub fn title(&self) -> Color {
        self.pick((238, 238, 238), Color::White)
    }
    /// Secondary text.
    pub fn dim(&self) -> Color {
        self.pick((128, 128, 128), Color::Gray)
    }
    /// Dividers, leaders and meter tracks.
    pub fn faint(&self) -> Color {
        self.pick((64, 64, 64), Color::DarkGray)
    }

    // --- Data ---

    /// Plain values.
    pub fn fg(&self) -> Color {
        self.pick((204, 204, 204), Color::White)
    }
    /// btop's highlight: hotkey letters and the selected row.
    pub fn hi(&self) -> Color {
        self.pick((225, 82, 82), Color::LightRed)
    }
    /// Background of the selected title, as btop's selected row.
    pub fn selected_bg(&self) -> Color {
        self.pick((106, 47, 47), Color::Red)
    }
    /// Warm light for lights that are on.
    pub fn glow(&self) -> Color {
        self.pick((255, 200, 80), Color::Yellow)
    }
    /// Peak of the "breathing" glow.
    pub fn glow_bright(&self) -> Color {
        self.pick((255, 236, 170), Color::LightYellow)
    }
    pub fn heat(&self) -> Color {
        self.pick((255, 110, 60), Color::LightRed)
    }
    pub fn cool(&self) -> Color {
        self.pick((92, 168, 240), Color::LightBlue)
    }

    // --- Status ---

    pub fn good(&self) -> Color {
        self.pick((80, 240, 149), Color::Green)
    }
    pub fn warn(&self) -> Color {
        self.pick((242, 226, 102), Color::Yellow)
    }
    pub fn bad(&self) -> Color {
        self.pick((250, 64, 64), Color::Red)
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
        assert_eq!(TC.temperature(0.0), Color::Rgb(72, 151, 212));
        assert_eq!(TC.temperature(40.0), Color::Rgb(250, 30, 30));
        assert_eq!(TC.temperature(20.0), Color::Rgb(80, 240, 149));
        assert_eq!(TC.graph(0.0), Color::Rgb(80, 240, 149));
        assert_eq!(TC.graph(1.0), Color::Rgb(250, 30, 30));
    }

    #[test]
    fn sixteen_colour_fallback_snaps_to_nearest_stop() {
        let t = Theme {
            truecolor: false,
            ascii: false,
        };
        assert_eq!(t.temperature(20.4), Color::Green);
        assert_eq!(t.temperature(22.8), Color::Yellow);
        assert_eq!(t.accent(Some("magenta"), 0), Color::Magenta);
        assert_eq!(t.frame(), Color::DarkGray);
    }
}

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
    pub depth: Depth,
    /// Bare Linux console: ASCII graphs (no braille glyphs).
    pub ascii: bool,
}

/// How many colours the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// 24-bit RGB: smooth gradients.
    Truecolor,
    /// The xterm 256-colour palette: gradients in coarser steps.
    Indexed,
    /// The 16 ANSI colours: two-tone graphs.
    Basic,
}

/// The palette: seven hues anchored on orange (its complement teal, triads
/// violet and gold, and rose, sky and sage to round it out), balanced to sit
/// together on black. Each panel takes one hue for its frame and graphs and
/// pairs it with white text.
///
/// (name, base colour, 16-colour dark/light pair)
pub const HUES: &[(&str, Rgb, (Color, Color))] = &[
    ("orange", (236, 138, 58), (Color::Yellow, Color::LightYellow)),
    ("teal", (64, 176, 170), (Color::Cyan, Color::LightCyan)),
    ("violet", (138, 116, 222), (Color::Magenta, Color::LightMagenta)),
    ("rose", (222, 96, 120), (Color::Red, Color::LightRed)),
    ("gold", (222, 186, 84), (Color::Yellow, Color::LightYellow)),
    ("sky", (92, 160, 226), (Color::Blue, Color::LightBlue)),
    ("sage", (132, 188, 112), (Color::Green, Color::LightGreen)),
];

/// Hue indices with fixed roles.
pub mod hue {
    pub const ORANGE: usize = 0;
    pub const GOLD: usize = 4;
    pub const SKY: usize = 5;
    pub const SAGE: usize = 6;
    pub const ROSE: usize = 3;
}

/// Graph gradients run from the hue's deep shade at the baseline to a light
/// tint at the top.
const GRADIENT_LOW: f64 = 0.38; // fraction of the base colour
const GRADIENT_HIGH: f64 = 0.5; // mix towards white

impl Theme {
    /// Detect the terminal's colour depth, unless the config forces one.
    pub fn detect(forced: Option<Depth>) -> Self {
        let var = |k| std::env::var(k).unwrap_or_default();
        let term = var("TERM");
        Self {
            depth: forced.unwrap_or_else(|| detect_depth(&term, &var("COLORTERM"), &var("TERM_PROGRAM"))),
            ascii: term == "linux",
        }
    }

    fn pick(&self, rgb: Rgb, fallback: Color) -> Color {
        match self.depth {
            Depth::Truecolor => Color::Rgb(rgb.0, rgb.1, rgb.2),
            Depth::Indexed => Color::Indexed(xterm256(rgb)),
            Depth::Basic => fallback,
        }
    }

    // --- Neutrals: white text, greys for structure ---

    /// Titles, labels and values: white.
    pub fn title(&self) -> Color {
        self.pick((242, 242, 242), Color::White)
    }
    /// Secondary text.
    pub fn dim(&self) -> Color {
        self.pick((142, 142, 142), Color::Gray)
    }
    /// Dividers, leaders and meter tracks.
    pub fn faint(&self) -> Color {
        self.pick((60, 60, 60), Color::DarkGray)
    }
    /// Plain values.
    pub fn fg(&self) -> Color {
        self.pick((222, 222, 222), Color::White)
    }

    // --- Pops of colour, all from the palette ---

    /// Hotkeys and selection: the anchor orange.
    pub fn hi(&self) -> Color {
        self.ink(hue::ORANGE)
    }
    /// Background of the selected title: deep orange.
    pub fn selected_bg(&self) -> Color {
        self.pick((112, 56, 20), Color::Yellow)
    }
    /// Lights that are on: gold.
    pub fn glow(&self) -> Color {
        self.ink(hue::GOLD)
    }
    /// Peak of the "breathing" glow.
    pub fn glow_bright(&self) -> Color {
        self.pick((250, 230, 170), Color::LightYellow)
    }
    /// Heating: orange.
    pub fn heat(&self) -> Color {
        self.ink(hue::ORANGE)
    }
    /// Cooling: sky.
    pub fn cool(&self) -> Color {
        self.ink(hue::SKY)
    }
    pub fn good(&self) -> Color {
        self.ink(hue::SAGE)
    }
    pub fn warn(&self) -> Color {
        self.ink(hue::GOLD)
    }
    pub fn bad(&self) -> Color {
        self.ink(hue::ROSE)
    }
    /// Lines drawn over graphs (setpoints): near-white, visible on any hue.
    pub fn overlay(&self) -> Color {
        self.pick((236, 236, 236), Color::White)
    }

    // --- Hues ---

    /// A hue by name, or the `index`th in the cycle.
    pub fn hue(&self, name: Option<&str>, index: usize) -> usize {
        name.and_then(|n| HUES.iter().position(|(h, _, _)| *h == n)).unwrap_or(index % HUES.len())
    }

    /// A hue's base colour (frames, titles' accents).
    pub fn ink(&self, h: usize) -> Color {
        let (_, rgb, basic) = HUES[h % HUES.len()];
        self.pick(rgb, basic.1)
    }

    /// A hue darkened towards black by `amount` (0..1), for quieter frames.
    pub fn shade(&self, h: usize, amount: f64) -> Color {
        let (_, (r, g, b), basic) = HUES[h % HUES.len()];
        let k = 1.0 - amount.clamp(0.0, 1.0);
        let s = |v: u8| (f64::from(v) * k).round() as u8;
        self.pick((s(r), s(g), s(b)), basic.0)
    }

    /// A hue's graph colour at a height fraction (0 = baseline): from its
    /// deep shade up to a light tint.
    pub fn graph(&self, h: usize, frac: f64) -> Color {
        let (_, (r, g, b), basic) = HUES[h % HUES.len()];
        let low = |v: u8| f64::from(v) * GRADIENT_LOW;
        let high = |v: u8| f64::from(v) + (255.0 - f64::from(v)) * GRADIENT_HIGH;
        let t = frac.clamp(0.0, 1.0);
        let c = |v: u8| (low(v) + (high(v) - low(v)) * t).round() as u8;
        self.pick((c(r), c(g), c(b)), if t < 0.5 { basic.0 } else { basic.1 })
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

}

/// Best guess at colour depth from the environment.
///
/// `COLORTERM=truecolor` is the standard signal, but several terminals with
/// 24-bit colour don't set it (and tmux often doesn't pass it through), so
/// known terminals are recognised too. Anything else advertising 256 colours
/// gets the 256-colour palette rather than falling all the way back to 16.
pub fn detect_depth(term: &str, colorterm: &str, term_program: &str) -> Depth {
    const TRUECOLOR_PROGRAMS: &[&str] = &["iTerm.app", "WezTerm", "vscode", "ghostty", "Hyper", "Tabby", "rio", "WarpTerminal"];
    const TRUECOLOR_TERMS: &[&str] = &["direct", "truecolor", "kitty", "alacritty", "ghostty", "wezterm", "foot"];
    if matches!(colorterm, "truecolor" | "24bit")
        || TRUECOLOR_PROGRAMS.contains(&term_program)
        || TRUECOLOR_TERMS.iter().any(|t| term.contains(t))
    {
        Depth::Truecolor
    } else if term.contains("256") {
        Depth::Indexed
    } else {
        Depth::Basic
    }
}

/// Nearest colour in the xterm 256-colour palette: the 6×6×6 cube or the
/// 24-step grey ramp, whichever is closer.
pub fn xterm256((r, g, b): Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let grey_i = ((u32::from(r) + u32::from(g) + u32::from(b)) / 3).saturating_sub(8) / 10;
    let grey_i = grey_i.min(23) as u8;
    let grey_v = 8 + 10 * grey_i;
    let dist = |(a, b2, c): (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a, r) + d(b2, g) + d(c, b)
    };
    if dist((grey_v, grey_v, grey_v)) < dist(cube) {
        232 + grey_i
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}


pub fn hue_names() -> impl Iterator<Item = &'static str> {
    HUES.iter().map(|(n, _, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TC: Theme = Theme {
        depth: Depth::Truecolor,
        ascii: false,
    };

    #[test]
    fn graphs_run_from_deep_shade_to_light_tint() {
        // Orange (236,138,58): ×0.38 at the base, halfway to white at the top.
        assert_eq!(TC.graph(hue::ORANGE, 0.0), Color::Rgb(90, 52, 22));
        assert_eq!(TC.graph(hue::ORANGE, 1.0), Color::Rgb(246, 197, 157));
        assert_eq!(TC.graph(hue::ORANGE, 7.0), TC.graph(hue::ORANGE, 1.0));
        assert_eq!(TC.hue(Some("violet"), 0), 2);
        assert_eq!(TC.hue(None, 8), 1, "cycles");
    }

    #[test]
    fn sixteen_colour_fallback() {
        let t = Theme {
            depth: Depth::Basic,
            ascii: false,
        };
        assert_eq!(t.graph(hue::SKY, 0.2), Color::Blue);
        assert_eq!(t.graph(hue::SKY, 0.8), Color::LightBlue, "two-tone, not flat");
        assert_eq!(t.ink(1), Color::LightCyan, "teal");
    }

    #[test]
    fn indexed_gradients_have_several_steps() {
        let t = Theme {
            depth: Depth::Indexed,
            ascii: false,
        };
        let shades: std::collections::BTreeSet<_> = (0..=10).map(|i| format!("{:?}", t.graph(2, i as f64 / 10.0))).collect();
        assert!(shades.len() >= 3, "{shades:?}");
    }

    #[test]
    fn xterm256_maps_cube_and_greys() {
        assert_eq!(xterm256((0, 0, 0)), 16);
        assert_eq!(xterm256((255, 255, 255)), 231);
        assert_eq!(xterm256((255, 0, 0)), 196);
        assert_eq!(xterm256((128, 128, 128)), 244);
    }

    #[test]
    fn detects_depth() {
        assert_eq!(detect_depth("xterm-256color", "truecolor", ""), Depth::Truecolor);
        assert_eq!(detect_depth("xterm-256color", "", "iTerm.app"), Depth::Truecolor);
        assert_eq!(detect_depth("xterm-256color", "", "Apple_Terminal"), Depth::Indexed);
        assert_eq!(detect_depth("tmux-256color", "", "tmux"), Depth::Indexed);
        assert_eq!(detect_depth("xterm-ghostty", "", ""), Depth::Truecolor);
        assert_eq!(detect_depth("xterm", "", ""), Depth::Basic);
    }
}

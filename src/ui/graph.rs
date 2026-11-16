//! btop-style graphs: filled braille area graphs coloured by height, with an
//! optional line overlay, and `■■■■····` meters.
//!
//! Braille cells are 2 dots wide and 4 tall, so a graph `w` cells wide takes
//! `2w` samples and has `4h` levels of vertical resolution. On the bare Linux
//! console (no braille glyphs) graphs fall back to ASCII at cell resolution.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// Dot bits for the left and right column of a braille cell, top to bottom.
const LEFT: [u8; 4] = [0x01, 0x02, 0x04, 0x40];
const RIGHT: [u8; 4] = [0x08, 0x10, 0x20, 0x80];

pub struct Graph<'a> {
    /// Samples oldest → newest; `None` draws nothing (no data yet).
    pub samples: &'a [Option<f64>],
    pub lo: f64,
    pub hi: f64,
    /// Colour for a cell row, given its height (0 = baseline, 1 = top).
    pub colour: &'a dyn Fn(f64) -> Color,
    /// A thin line (e.g. a thermostat setpoint), with its colour. Above the
    /// fill it's drawn as dots; through the fill it's carved as a one-dot gap,
    /// so it reads as one continuous line.
    pub overlay: Option<(&'a [Option<f64>], Color)>,
    pub ascii: bool,
}

impl Graph<'_> {
    /// How many samples to provide for a graph `width` cells wide.
    pub fn samples_for(width: u16, ascii: bool) -> usize {
        if ascii { width as usize } else { width as usize * 2 }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() || self.hi <= self.lo {
            return;
        }
        if self.ascii {
            self.render_ascii(area, buf);
            return;
        }
        let rows = area.height as usize * 4;
        // Dot level (0..=rows) for a value; values fill from the baseline.
        let level = |v: f64| (((v - self.lo) / (self.hi - self.lo)) * rows as f64).round().clamp(0.0, rows as f64) as usize;
        // The overlay is a single dot per sample, one below the filled top.
        let line_level = |v: f64| level(v).max(1) - 1;

        for cy in 0..area.height as usize {
            // Dot rows covered by this cell, counted from the baseline.
            let base = (area.height as usize - 1 - cy) * 4;
            let frac = (base as f64 + 2.0) / rows as f64;
            for cx in 0..area.width as usize {
                let mut bits = 0u8;
                let mut line_above_fill = false;
                for (side, dots) in [(0, LEFT), (1, RIGHT)] {
                    let s = cx * 2 + side;
                    let fill = self.samples.get(s).copied().flatten().map(level);
                    let line = self.overlay.and_then(|(o, _)| o.get(s).copied().flatten()).map(line_level);
                    for (dy, bit) in dots.iter().enumerate() {
                        let dot = base + (3 - dy); // dot row from the baseline
                        let filled = fill.is_some_and(|f| dot < f);
                        if line == Some(dot) {
                            if filled {
                                continue; // carve a gap through the fill
                            }
                            bits |= bit;
                            line_above_fill = true;
                        } else if filled {
                            bits |= bit;
                        }
                    }
                }
                if bits == 0 {
                    continue;
                }
                let colour = match self.overlay {
                    Some((_, c)) if line_above_fill => c,
                    _ => (self.colour)(frac),
                };
                let ch = char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ');
                buf[(area.x + cx as u16, area.y + cy as u16)]
                    .set_char(ch)
                    .set_style(Style::new().fg(colour));
            }
        }
    }

    fn render_ascii(&self, area: Rect, buf: &mut Buffer) {
        let rows = area.height as usize;
        for cx in 0..area.width as usize {
            let Some(v) = self.samples.get(cx).copied().flatten() else { continue };
            let filled = (((v - self.lo) / (self.hi - self.lo)) * rows as f64).round().clamp(0.0, rows as f64) as usize;
            for cy in 0..rows {
                let from_base = rows - 1 - cy;
                if from_base < filled {
                    let frac = (from_base as f64 + 0.5) / rows as f64;
                    buf[(area.x + cx as u16, area.y + cy as u16)]
                        .set_char(if from_base + 1 == filled { '.' } else { ':' })
                        .set_style(Style::new().fg((self.colour)(frac)));
                }
            }
        }
    }
}

/// A btop-style meter: `■` filled up to `frac`, coloured along the bar by
/// `colour(position)`, over a faint track.
pub fn meter(area: Rect, buf: &mut Buffer, frac: f64, colour: &dyn Fn(f64) -> Color, track: Color, ascii: bool) {
    if area.width == 0 {
        return;
    }
    let (on, off) = if ascii { ('#', '-') } else { ('■', '■') };
    let filled = (frac.clamp(0.0, 1.0) * f64::from(area.width)).round() as u16;
    for i in 0..area.width {
        let pos = (f64::from(i) + 0.5) / f64::from(area.width);
        let (ch, c) = if i < filled { (on, colour(pos)) } else { (off, track) };
        buf[(area.x + i, area.y)].set_char(ch).set_style(Style::new().fg(c));
    }
}

/// A sensible value range for `samples`: at least `min_span` wide, padded a
/// little so the top of the fill doesn't touch the frame.
pub fn range(samples: impl IntoIterator<Item = f64>, min_span: f64) -> Option<(f64, f64)> {
    let (lo, hi) = samples
        .into_iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if !lo.is_finite() {
        return None;
    }
    let (mut lo, mut hi) = (lo, hi);
    if hi - lo < min_span {
        let mid = (hi + lo) / 2.0;
        lo = mid - min_span / 2.0;
        hi = mid + min_span / 2.0;
    }
    let pad = (hi - lo) * 0.08;
    Some((lo - pad, hi + pad))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(g: &Graph, w: u16, h: u16) -> String {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        g.render(Rect::new(0, 0, w, h), &mut buf);
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn fills_from_the_baseline() {
        let samples: Vec<Option<f64>> = (0..8).map(|i| Some(i as f64)).collect();
        let g = Graph {
            samples: &samples,
            lo: 0.0,
            hi: 8.0,
            colour: &|_| Color::Reset,
            overlay: None,
            ascii: false,
        };
        // Samples 0..7 fill 0..7 dots of an 8-dot-tall graph.
        insta::assert_snapshot!(draw(&g, 4, 2), @r"
          ⢀⣴
        ⢀⣴⣿⣿
        ");
    }

    #[test]
    fn overlay_is_dots_above_and_a_gap_through_the_fill() {
        let samples = [None, None, Some(4.0), Some(4.0)];
        // Line at 2 is inside the right cell's fill (level 4).
        let line = [Some(2.0); 4];
        let g = Graph {
            samples: &samples,
            lo: 0.0,
            hi: 8.0,
            colour: &|_| Color::Reset,
            overlay: Some((&line, Color::Red)),
            ascii: false,
        };
        // Left: dots where there's no data. Right: full cell minus the line's
        // dot row (⣿ with ⠤ removed is ⣛).
        assert_eq!(draw(&g, 2, 2), "  \n⠤⣛");
        // A line above the fill is drawn as dots.
        let high = [Some(6.0); 4];
        let g = Graph { overlay: Some((&high, Color::Red)), ..g };
        assert_eq!(draw(&g, 2, 2), "⠤⠤\n ⣿");
    }

    #[test]
    fn ascii_fallback() {
        let samples = [Some(1.0), Some(4.0)];
        let g = Graph {
            samples: &samples,
            lo: 0.0,
            hi: 4.0,
            colour: &|_| Color::Reset,
            overlay: None,
            ascii: true,
        };
        assert_eq!(draw(&g, 2, 2), " .\n.:");
    }

    #[test]
    fn meter_fills_proportionally() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        meter(Rect::new(0, 0, 10, 1), &mut buf, 0.3, &|_| Color::Red, Color::DarkGray, true);
        let s: String = (0..10).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(s, "###-------");
    }

    #[test]
    fn range_has_minimum_span_and_padding() {
        let (lo, hi) = range([20.0, 20.0], 2.0).unwrap();
        assert!(lo < 19.0 && hi > 21.0);
        assert!(range(std::iter::empty(), 1.0).is_none());
    }
}

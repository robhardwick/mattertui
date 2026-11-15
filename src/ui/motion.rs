//! Small, hand-rolled animation helpers: easing, tweens, colour mixing and
//! the event ticker. Everything is driven by wall-clock time, so frame rate
//! only affects smoothness, never speed.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::style::Color;

pub const TWEEN: Duration = Duration::from_millis(700);
pub const FLASH: Duration = Duration::from_millis(1200);
/// Delay between rooms sweeping in at startup, and each sweep's length.
pub const SWEEP_STAGGER: Duration = Duration::from_millis(110);
pub const SWEEP: Duration = Duration::from_millis(450);
const TICKER_CAPACITY: usize = 30;

/// 0..=1 progress of an animation that started at `start`.
pub fn progress(start: Instant, len: Duration, now: Instant) -> f64 {
    (now.saturating_duration_since(start).as_secs_f64() / len.as_secs_f64()).clamp(0.0, 1.0)
}

pub fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// A gentle 0..=1 "breathing" wave with the given period.
pub fn breathe(epoch: Instant, period: Duration, now: Instant) -> f64 {
    let phase = now.saturating_duration_since(epoch).as_secs_f64() / period.as_secs_f64();
    0.5 - 0.5 * (phase * std::f64::consts::TAU).cos()
}

/// Mix two colours; `t` = 0 gives `a`. Non-RGB colours snap at the midpoint.
pub fn mix(a: Color, b: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let l = |x: u8, y: u8| (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
            Color::Rgb(l(r1, r2), l(g1, g2), l(b1, b2))
        }
        _ if t < 0.5 => a,
        _ => b,
    }
}

/// A number animating from one value to another.
#[derive(Debug, Clone, Copy)]
pub struct Tween {
    pub from: f64,
    pub to: f64,
    pub start: Instant,
}

impl Tween {
    pub fn value(&self, now: Instant) -> f64 {
        let t = ease_out_cubic(progress(self.start, TWEEN, now));
        self.from + (self.to - self.from) * t
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= TWEEN
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Glow,
    Heat,
    Good,
    Bad,
}

#[derive(Debug, Clone)]
pub struct TickerEvent {
    pub at: Instant,
    pub device: usize,
    pub text: String,
    pub tone: Tone,
}

/// Recent notable changes, newest first.
#[derive(Debug, Default)]
pub struct Ticker {
    events: VecDeque<TickerEvent>,
}

impl Ticker {
    pub fn push(&mut self, device: usize, text: String, tone: Tone) {
        self.events.push_front(TickerEvent {
            at: Instant::now(),
            device,
            text,
            tone,
        });
        self.events.truncate(TICKER_CAPACITY);
    }

    pub fn events(&self) -> impl Iterator<Item = &TickerEvent> {
        self.events.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_eases_from_start_to_end() {
        let start = Instant::now();
        let tw = Tween {
            from: 18.0,
            to: 20.0,
            start,
        };
        assert_eq!(tw.value(start), 18.0);
        let mid = tw.value(start + TWEEN / 2);
        assert!(mid > 19.0 && mid < 20.0, "ease-out is past halfway at half time: {mid}");
        assert_eq!(tw.value(start + TWEEN * 2), 20.0);
        assert!(tw.done(start + TWEEN));
    }

    #[test]
    fn mixes_rgb_and_snaps_named() {
        assert_eq!(mix(Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 50), 0.5), Color::Rgb(100, 50, 25));
        assert_eq!(mix(Color::Red, Color::Blue, 0.4), Color::Red);
        assert_eq!(mix(Color::Red, Color::Blue, 0.6), Color::Blue);
    }

    #[test]
    fn ticker_is_newest_first_and_bounded() {
        let mut t = Ticker::default();
        for i in 0..40 {
            t.push(0, format!("e{i}"), Tone::Normal);
        }
        let texts: Vec<_> = t.events().map(|e| e.text.as_str()).collect();
        assert_eq!(texts.len(), TICKER_CAPACITY);
        assert_eq!(texts[0], "e39");
    }
}

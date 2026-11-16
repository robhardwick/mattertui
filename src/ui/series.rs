//! Turning history buckets into things to draw.

use crate::state::History;

/// Sample `history` into `n` evenly spaced columns covering the last
/// `range_min` minutes up to `now_min`; `None` before the first bucket.
///
/// Between two buckets, `continuous` metrics (temperature, humidity, …) are
/// interpolated linearly, which also bridges gaps such as the app not
/// running. Others (on/off) hold the earlier value, as a step. After the
/// last bucket the latest value is carried forward to now.
pub fn sample(history: &History, now_min: i64, range_min: i64, n: usize, continuous: bool) -> Vec<Option<f64>> {
    if n == 0 {
        return Vec::new();
    }
    let buckets: Vec<_> = history.buckets().collect();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    let mut current: Option<f64> = None;
    for col in 0..n {
        // Column end time, rounded down to whole minutes.
        let t = now_min - range_min + ((col as i64 + 1) * range_min) / n as i64;
        while i < buckets.len() && buckets[i].minute <= t {
            current = Some(buckets[i].last);
            i += 1;
        }
        let v = match (current, buckets.get(i)) {
            (Some(v), Some(next)) if continuous && i > 0 => {
                let prev = buckets[i - 1];
                let f = (t - prev.minute) as f64 / (next.minute - prev.minute) as f64;
                Some(v + (next.last - v) * f)
            }
            _ => current,
        };
        out.push(v);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

pub fn stats(history: &History, now_min: i64, range_min: i64) -> Option<Stats> {
    let start = now_min - range_min;
    let mut n = 0.0;
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for b in history.buckets().filter(|b| b.minute >= start) {
        min = min.min(b.min);
        max = max.max(b.max);
        sum += b.last;
        n += 1.0;
    }
    (n > 0.0).then(|| Stats {
        min,
        max,
        avg: sum / n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(points: &[(i64, f64)]) -> History {
        let mut h = History::new(24 * 60);
        for &(m, v) in points {
            h.record(m, v);
        }
        h
    }

    #[test]
    fn sample_carries_values_forward() {
        let h = hist(&[(100, 1.0), (105, 2.0)]);
        // 10 columns over minutes 101..=110, one per minute: a step...
        let s = sample(&h, 110, 10, 10, false);
        assert_eq!(s, [1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0].map(Some));
        // ...or a ramp between the buckets, then carried forward.
        let s = sample(&h, 110, 10, 10, true);
        assert_eq!(s, [1.2, 1.4, 1.6, 1.8, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0].map(Some));
        // Before any data → None.
        let s = sample(&h, 100, 10, 2, true);
        assert_eq!(s, [None, Some(1.0)]);
    }

    #[test]
    fn stats_over_window_only() {
        let h = hist(&[(50, 100.0), (105, 2.0), (106, 4.0)]);
        let s = stats(&h, 110, 10).unwrap();
        assert_eq!((s.min, s.max, s.avg), (2.0, 4.0, 3.0));
        assert!(stats(&h, 300, 10).is_none());
    }
}

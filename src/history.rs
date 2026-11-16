//! Graph history that survives restarts: one JSONL file per device in
//! `$XDG_CACHE_HOME/mattertui/history/`, one line per completed minute per
//! metric:
//!
//! ```text
//! {"m":"temperature","t":29400123,"lo":19.25,"hi":19.31,"v":19.28}
//! ```
//!
//! Files are append-only while running (a line a minute per metric, so a few
//! hundred KB per device per day). On load they're trimmed to the history
//! window and rewritten. Gaps (e.g. while the app wasn't running) are
//! interpolated when drawn, not stored.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::{Bucket, History, Metric, unix_minute};

#[derive(Serialize, Deserialize)]
struct Line {
    m: Metric,
    t: i64,
    lo: f64,
    hi: f64,
    v: f64,
}

pub struct HistoryStore {
    dir: PathBuf,
    capacity_minutes: usize,
}

impl HistoryStore {
    pub fn open(dir: PathBuf, capacity_minutes: usize) -> Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir, capacity_minutes })
    }

    fn path(&self, device: &str) -> PathBuf {
        self.dir.join(format!("{device}.jsonl"))
    }

    /// Load a device's history within the window, and rewrite its file with
    /// only those lines. Unreadable lines are skipped.
    pub fn load(&self, device: &str) -> Result<BTreeMap<Metric, History>> {
        let path = self.path(device);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let now = unix_minute();
        let oldest = now - self.capacity_minutes as i64 + 1;
        let mut kept: Vec<Line> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<Line>(&l).ok())
            .filter(|l| l.t >= oldest && l.t <= now)
            .collect();
        kept.sort_by_key(|l| l.t);

        let mut out: BTreeMap<Metric, History> = BTreeMap::new();
        for l in &kept {
            out.entry(l.m).or_insert_with(|| History::new(self.capacity_minutes)).insert(Bucket {
                minute: l.t,
                min: l.lo,
                max: l.hi,
                last: l.v,
            });
        }
        for h in out.values_mut() {
            h.trim(now);
        }

        // Compact: rewrite atomically, one line per metric per minute (lines
        // for the same minute, e.g. from a quick restart, are merged).
        let tmp = path.with_extension("jsonl.tmp");
        {
            let mut f = std::io::BufWriter::new(File::create(&tmp)?);
            for (m, h) in &out {
                for b in h.buckets() {
                    let line = Line { m: *m, t: b.minute, lo: b.min, hi: b.max, v: b.last };
                    writeln!(f, "{}", serde_json::to_string(&line)?)?;
                }
            }
            f.flush()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(out)
    }

    /// Append one completed minute.
    pub fn append(&self, device: &str, metric: Metric, b: &Bucket) -> Result<()> {
        let mut f = OpenOptions::new().create(true).append(true).open(self.path(device))?;
        let line = Line {
            m: metric,
            t: b.minute,
            lo: b.min,
            hi: b.max,
            v: b.last,
        };
        writeln!(f, "{}", serde_json::to_string(&line)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mattertui-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trips_and_trims_to_the_window() {
        let dir = temp_dir("roundtrip");
        let store = HistoryStore::open(dir.clone(), 60).unwrap();
        let now = unix_minute();
        let b = |minute, v| Bucket { minute, min: v, max: v + 1.0, last: v };
        store.append("lamp", Metric::Temperature, &b(now - 120, 1.0)).unwrap(); // too old
        store.append("lamp", Metric::Temperature, &b(now - 10, 2.0)).unwrap();
        store.append("lamp", Metric::Humidity, &b(now - 5, 50.0)).unwrap();
        store.append("lamp", Metric::Humidity, &b(now - 5, 52.0)).unwrap(); // same minute again
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("lamp.jsonl"))
            .unwrap()
            .write_all(b"not json\n")
            .unwrap();

        let h = store.load("lamp").unwrap();
        let temps: Vec<_> = h[&Metric::Temperature].buckets().copied().collect();
        assert_eq!(temps, vec![b(now - 10, 2.0)]);
        let hum: Vec<_> = h[&Metric::Humidity].buckets().copied().collect();
        assert_eq!(hum, vec![Bucket { minute: now - 5, min: 50.0, max: 53.0, last: 52.0 }], "merged");

        // The file was compacted: old and bad lines are gone.
        let text = fs::read_to_string(dir.join("lamp.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains(r#""m":"temperature""#));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_empty_history() {
        let store = HistoryStore::open(temp_dir("missing"), 60).unwrap();
        assert!(store.load("nobody").unwrap().is_empty());
    }
}

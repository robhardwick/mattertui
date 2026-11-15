//! Device state as seen by the app, plus in-memory metric history.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A value a device reports. Booleans and enums are stored as numbers so
/// every metric shares one history representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Metric {
    /// Plug on/off (1.0 / 0.0).
    On,
    /// °C.
    Temperature,
    /// %RH.
    Humidity,
    /// °C.
    HeatSetpoint,
    /// °C.
    CoolSetpoint,
    /// Thermostat SystemMode enum value.
    Mode,
    /// ThermostatRunningState bitmap (bit 0 heat, bit 1 cool, bit 2 fan).
    Running,
    /// Battery %, 0–100.
    Battery,
}

impl Metric {
    /// Metrics that get a history ring (and so a graph).
    pub fn graphed(self) -> bool {
        !matches!(self, Metric::Mode)
    }
}

/// Everything that flows into the app loop.
#[derive(Debug)]
pub enum AppEvent {
    Device { id: String, update: DeviceUpdate },
    /// Commissioning finished: the new node id, or an error message.
    Paired { id: String, result: Result<u64, String> },
    /// Unpairing finished: a note for the user (e.g. "removed from device").
    Unpaired { id: String, result: Result<String, String> },
}

/// One update from a device worker.
#[derive(Debug, Clone)]
pub enum DeviceUpdate {
    Status(Status),
    Info(DeviceInfo),
    /// `None` when the device reports null (e.g. sensor not ready).
    Reading { metric: Metric, value: Option<f64> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// In config, but not paired yet.
    Unpaired,
    /// Probing / subscribing.
    Connecting,
    /// Subscription established; the device promises a report at least every
    /// `max_interval_s`.
    Live { max_interval_s: u16 },
    /// Subscription lost; the library is re-establishing it.
    Reconnecting { cause: String },
    /// Could not reach the device; retrying after a backoff.
    Offline { error: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    pub node_id: u64,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub firmware: Option<String>,
    pub transport: Transport,
    pub endpoint: u16,
    /// `Some` for intermittently connected (battery) devices.
    pub icd: Option<IcdInfo>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Transport {
    #[default]
    Unknown,
    WiFi,
    Thread,
    Ethernet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IcdInfo {
    pub long_idle_time: bool,
    pub check_in: bool,
    pub idle_mode_s: Option<u32>,
}

/// Everything the UI knows about one configured device.
#[derive(Debug)]
pub struct DeviceState {
    pub status: Status,
    pub info: Option<DeviceInfo>,
    pub current: BTreeMap<Metric, Option<f64>>,
    pub history: BTreeMap<Metric, History>,
    pub last_report: Option<Instant>,
    /// When `status` last changed to a different kind of status.
    pub status_since: Instant,
    history_minutes: usize,
}

impl DeviceState {
    pub fn new(history_hours: u32) -> Self {
        Self {
            status: Status::Unpaired,
            info: None,
            current: BTreeMap::new(),
            history: BTreeMap::new(),
            last_report: None,
            status_since: Instant::now(),
            history_minutes: history_hours as usize * 60,
        }
    }

    pub fn apply(&mut self, update: DeviceUpdate) {
        match update {
            DeviceUpdate::Status(s) => {
                if std::mem::discriminant(&s) != std::mem::discriminant(&self.status) {
                    self.status_since = Instant::now();
                }
                self.status = s;
            }
            DeviceUpdate::Info(i) => self.info = Some(i),
            DeviceUpdate::Reading { metric, value } => {
                self.last_report = Some(Instant::now());
                self.current.insert(metric, value);
                if let (Some(v), true) = (value, metric.graphed()) {
                    self.history
                        .entry(metric)
                        .or_insert_with(|| History::new(self.history_minutes))
                        .record(unix_minute(), v);
                }
            }
        }
    }
}

/// Min/max/last of one metric within one wall-clock minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bucket {
    /// Minutes since the Unix epoch.
    pub minute: i64,
    pub min: f64,
    pub max: f64,
    pub last: f64,
}

/// Fixed-capacity ring of 1-minute buckets. Minutes with no report have no
/// bucket; charts treat the gap as "value unchanged".
#[derive(Debug, Clone)]
pub struct History {
    buckets: VecDeque<Bucket>,
    capacity_minutes: usize,
}

impl History {
    pub fn new(capacity_minutes: usize) -> Self {
        Self {
            buckets: VecDeque::new(),
            capacity_minutes,
        }
    }

    pub fn record(&mut self, minute: i64, value: f64) {
        match self.buckets.back_mut() {
            Some(b) if b.minute == minute => {
                b.min = b.min.min(value);
                b.max = b.max.max(value);
                b.last = value;
            }
            _ => self.buckets.push_back(Bucket {
                minute,
                min: value,
                max: value,
                last: value,
            }),
        }
        let oldest_kept = minute - self.capacity_minutes as i64 + 1;
        while self.buckets.front().is_some_and(|b| b.minute < oldest_kept) {
            self.buckets.pop_front();
        }
    }

    pub fn buckets(&self) -> impl Iterator<Item = &Bucket> {
        self.buckets.iter()
    }

    #[allow(dead_code)] // used by the UI
    pub fn len(&self) -> usize {
        self.buckets.len()
    }
}

/// Minutes since the Unix epoch (history bucket key).
pub fn unix_minute() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs() / 60) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_minute_merges_into_one_bucket() {
        let mut h = History::new(60);
        h.record(100, 20.0);
        h.record(100, 22.0);
        h.record(100, 21.0);
        let b: Vec<_> = h.buckets().copied().collect();
        assert_eq!(
            b,
            vec![Bucket {
                minute: 100,
                min: 20.0,
                max: 22.0,
                last: 21.0
            }]
        );
    }

    #[test]
    fn old_buckets_age_out_by_time_not_count() {
        let mut h = History::new(10);
        h.record(100, 1.0);
        h.record(105, 2.0);
        h.record(109, 3.0);
        assert_eq!(h.len(), 3);
        h.record(110, 4.0); // window is now 101..=110, so minute 100 goes
        assert_eq!(h.buckets().map(|b| b.minute).collect::<Vec<_>>(), [105, 109, 110]);
        h.record(200, 5.0);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn null_readings_update_current_but_not_history() {
        let mut s = DeviceState::new(1);
        s.apply(DeviceUpdate::Reading {
            metric: Metric::Temperature,
            value: None,
        });
        assert_eq!(s.current.get(&Metric::Temperature), Some(&None));
        assert!(s.history.is_empty());
        s.apply(DeviceUpdate::Reading {
            metric: Metric::Temperature,
            value: Some(21.5),
        });
        assert_eq!(s.history[&Metric::Temperature].len(), 1);
    }
}

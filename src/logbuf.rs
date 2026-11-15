//! A `tracing` layer that keeps recent log lines in memory for the
//! dashboard's log panel. Nothing is written to disk.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    pub target: String,
    pub message: String,
    /// The `device` field, if the event carried one.
    pub device: Option<String>,
    /// Remaining fields, rendered `k=v k=v`.
    pub fields: String,
}

#[derive(Default)]
pub struct LogBuffer {
    lines: Mutex<VecDeque<LogLine>>,
    /// Signalled on every new line so the UI redraws only when needed.
    pub notify: Notify,
    /// Running count of WARN/ERROR lines, for the "unseen" badge.
    warnings: AtomicU64,
}

impl LogBuffer {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn push(&self, line: LogLine) {
        if line.level <= Level::WARN {
            self.warnings.fetch_add(1, Ordering::Relaxed);
        }
        let mut lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        if lines.len() == CAPACITY {
            lines.pop_front();
        }
        lines.push_back(line);
        drop(lines);
        self.notify.notify_one();
    }

    pub fn warnings(&self) -> u64 {
        self.warnings.load(Ordering::Relaxed)
    }

    /// Copy of the buffered lines, oldest first.
    pub fn snapshot(&self) -> Vec<LogLine> {
        let lines = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        lines.iter().cloned().collect()
    }

    /// The last `n` lines as plain text (printed after the TUI exits on error).
    pub fn tail_text(&self, n: usize) -> String {
        let lines = self.snapshot();
        let mut out = String::new();
        for l in lines.iter().skip(lines.len().saturating_sub(n)) {
            let _ = write!(out, "{} {:>5} ", l.time, l.level);
            if let Some(d) = &l.device {
                let _ = write!(out, "[{d}] ");
            }
            let _ = writeln!(out, "{} {}", l.message, l.fields);
        }
        out
    }

    /// The tracing layer that feeds this buffer.
    pub fn layer(self: &Arc<Self>) -> BufferLayer {
        BufferLayer(self.clone())
    }
}

pub struct BufferLayer(Arc<LogBuffer>);

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut v = Visitor::default();
        event.record(&mut v);
        let meta = event.metadata();
        self.0.push(LogLine {
            time: jiff::ZonedDateTime::now().strftime("%H:%M:%S").to_string(),
            level: *meta.level(),
            target: short_target(meta.target()),
            message: v.message,
            device: v.device,
            fields: v.fields.trim_start().to_string(),
        });
    }
}

/// `matter_controller::actor` → `matter_controller`.
fn short_target(t: &str) -> String {
    t.split("::").next().unwrap_or(t).to_string()
}

#[derive(Default)]
struct Visitor {
    message: String,
    device: Option<String>,
    fields: String,
}

impl Visit for Visitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "device" => self.device = Some(value.to_string()),
            name => {
                let _ = write!(self.fields, " {name}={value}");
            }
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "device" => self.device = Some(format!("{value:?}").trim_matches('"').to_string()),
            name => {
                let _ = write!(self.fields, " {name}={value:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn captures_message_device_and_fields() {
        let buf = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buf.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(device = "lamp", max_interval_s = 60, "subscribed");
            tracing::warn!("careful");
        });
        let lines = buf.snapshot();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].message, "subscribed");
        assert_eq!(lines[0].device.as_deref(), Some("lamp"));
        assert_eq!(lines[0].fields, "max_interval_s=60");
        assert_eq!(buf.warnings(), 1);
        assert!(buf.tail_text(1).contains("careful"));
    }
}

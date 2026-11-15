//! The app loop. Until the dashboard lands (milestone 3) it logs readings.

use std::collections::BTreeMap;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::matter::readonly::ReadOnlyNode;
use crate::matter::worker::{self, WorkerSpec};
use crate::matter::{self, Trust};
use crate::paths::Paths;
use crate::state::{AppEvent, DeviceState, DeviceUpdate, Metric};

pub async fn run(paths: &Paths, config: &Config) -> Result<()> {
    let _lock = paths.lock()?;
    let ctl = matter::open(paths, Trust::NotNeeded).await?;
    let (tx, mut rx) = mpsc::channel::<AppEvent>(1024);

    let mut devices: BTreeMap<String, DeviceState> = config
        .devices
        .iter()
        .map(|d| (d.id.get_ref().clone(), DeviceState::new(config.ui.history_hours)))
        .collect();

    let nodes = ctl.inner.nodes().await?;
    for dev in &config.devices {
        let id = dev.id.get_ref();
        match nodes.iter().find(|n| n.label.as_deref() == Some(id.as_str())) {
            Some(n) => worker::spawn(
                WorkerSpec {
                    device_id: id.clone(),
                    kind: dev.kind,
                    endpoint: dev.endpoint,
                },
                ReadOnlyNode::new(ctl.inner.node(n.node_id)),
                tx.clone(),
            ),
            None => tracing::warn!(device = id, "not paired; run `mattertui pair {id} <code>`"),
        }
    }
    for n in &nodes {
        let label = n.label.as_deref().unwrap_or("");
        if config.device(label).is_none() {
            tracing::warn!(
                node_id = format_args!("0x{:016X}", n.node_id),
                label,
                "paired device has no [[device]] entry in the config"
            );
        }
    }
    drop(tx);

    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Some(AppEvent::Device { id, update }) => {
                    log_update(&id, &update);
                    if let Some(state) = devices.get_mut(&id) {
                        state.apply(update);
                    }
                }
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

fn log_update(id: &str, update: &DeviceUpdate) {
    match update {
        DeviceUpdate::Reading { metric, value } => {
            tracing::info!(device = id, "{}", format_reading(*metric, *value));
        }
        DeviceUpdate::Info(info) => tracing::info!(
            device = id,
            "{} {} via {:?}, endpoint {}{}",
            info.vendor.as_deref().unwrap_or("?"),
            info.product.as_deref().unwrap_or("?"),
            info.transport,
            info.endpoint,
            info.icd.map_or(String::new(), |i| format!(", ICD {i:?}")),
        ),
        DeviceUpdate::Status(_) => {} // the worker logs status changes itself
    }
}

pub fn format_reading(metric: Metric, value: Option<f64>) -> String {
    let Some(v) = value else {
        return format!("{metric:?}: unknown");
    };
    match metric {
        Metric::On => (if v != 0.0 { "on" } else { "off" }).to_string(),
        Metric::Temperature => format!("temperature {v:.2}°C"),
        Metric::HeatSetpoint => format!("heat setpoint {v:.1}°C"),
        Metric::CoolSetpoint => format!("cool setpoint {v:.1}°C"),
        Metric::Humidity => format!("humidity {v:.1}%"),
        Metric::Battery => format!("battery {v:.0}%"),
        Metric::Mode => format!("mode {v}"),
        Metric::Running => format!("running 0b{:03b}", v as u64),
    }
}

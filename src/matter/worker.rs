//! One task per paired device: discover → subscribe → decode → AppEvent.

use std::time::Duration;

use anyhow::{Context, Result};
use matter_controller::{ReadPath, SubscriptionEvent};
use tokio::sync::mpsc;

use super::decode;
use super::discover::discover;
use super::readonly::ReadOnlyNode;
use crate::config::DeviceKind;
use crate::state::{AppEvent, DeviceUpdate, Status};

const MIN_INTERVAL_S: u16 = 1;
/// Ceiling we ask for; ICDs may negotiate something longer.
const MAX_INTERVAL_S: u16 = 60;
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(300);

pub struct WorkerSpec {
    pub device_id: String,
    pub kind: DeviceKind,
    pub endpoint: Option<u16>,
}

/// Start a worker. Aborting the handle drops its subscription.
pub fn spawn(spec: WorkerSpec, node: ReadOnlyNode, tx: mpsc::Sender<AppEvent>) -> tokio::task::AbortHandle {
    tokio::spawn(run(spec, node, tx)).abort_handle()
}

async fn run(spec: WorkerSpec, node: ReadOnlyNode, tx: mpsc::Sender<AppEvent>) {
    let send = Sender {
        id: spec.device_id.clone(),
        tx,
    };
    let mut backoff = BACKOFF_START;
    loop {
        send.status(Status::Connecting).await;
        match stream(&spec, &node, &send, &mut backoff).await {
            // Subscription ended cleanly: the controller is shutting down.
            Ok(()) => return,
            Err(e) => {
                let error = format!("{e:#}");
                tracing::warn!(device = spec.device_id, %error, retry_in = ?backoff, "device unavailable");
                send.status(Status::Offline { error }).await;
                if send.tx.is_closed() {
                    return;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(BACKOFF_MAX);
            }
        }
    }
}

async fn stream(
    spec: &WorkerSpec,
    node: &ReadOnlyNode,
    send: &Sender,
    backoff: &mut Duration,
) -> Result<()> {
    let found = discover(node, spec.kind, spec.endpoint)
        .await
        .context("discovering device")?;
    tracing::debug!(device = spec.device_id, paths = found.paths.len(), info = ?found.info, "discovered");
    send.update(DeviceUpdate::Info(found.info)).await;

    let mut sub = node
        .subscribe(&found.paths, MIN_INTERVAL_S, MAX_INTERVAL_S)
        .await
        .context("subscribing")?;

    // Resubscribes are routine; log them quietly.
    let mut resubscribed = false;
    while let Some(event) = sub.next().await {
        match event {
            SubscriptionEvent::Report(r) => {
                if let Some((metric, value)) =
                    decode::reading(r.path.cluster, r.path.attribute, &r.value)
                {
                    send.update(DeviceUpdate::Reading { metric, value }).await;
                }
            }
            SubscriptionEvent::Established { max_interval, .. } => {
                *backoff = BACKOFF_START;
                if resubscribed {
                    tracing::debug!(device = spec.device_id, max_interval_s = max_interval, "resubscribed");
                } else {
                    tracing::info!(device = spec.device_id, max_interval_s = max_interval, "subscribed");
                }
                send.status(Status::Live {
                    max_interval_s: max_interval,
                })
                .await;
            }
            SubscriptionEvent::Resubscribing { cause } => {
                resubscribed = true;
                tracing::debug!(device = spec.device_id, %cause, "resubscribing");
                send.status(Status::Reconnecting {
                    cause: cause.to_string(),
                })
                .await;
            }
            SubscriptionEvent::Lagged { dropped } => {
                tracing::warn!(device = spec.device_id, dropped, "reports dropped; re-reading");
                reread(node, &found.paths, send).await;
            }
            _ => {}
        }
        if send.tx.is_closed() {
            return Ok(());
        }
    }
    Ok(())
}

async fn reread(node: &ReadOnlyNode, paths: &[ReadPath], send: &Sender) {
    match node.read(paths).await {
        Ok(report) => {
            for (path, value) in &report {
                if let Some((metric, value)) = decode::reading(path.cluster, path.attribute, value) {
                    send.update(DeviceUpdate::Reading { metric, value }).await;
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "re-read failed"),
    }
}

struct Sender {
    id: String,
    tx: mpsc::Sender<AppEvent>,
}

impl Sender {
    async fn update(&self, update: DeviceUpdate) {
        // A closed channel means the app is exiting; nothing to do.
        let _ = self
            .tx
            .send(AppEvent::Device {
                id: self.id.clone(),
                update,
            })
            .await;
    }

    async fn status(&self, status: Status) {
        self.update(DeviceUpdate::Status(status)).await;
    }
}

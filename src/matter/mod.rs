//! Controller bootstrap shared by every command that talks to devices.

pub mod decode;
pub mod discover;
pub mod pairing;
pub mod probe;
pub mod readonly;
pub mod worker;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use matter_controller::{
    AttestationTrust, FabricConfig, FileStore, MatterController, MatterTime, NodeInfo,
};

use crate::paths::Paths;

/// Our single fabric. Its root key is unique to this store, so a fixed id
/// cannot collide with Apple Home's fabric.
const FABRIC_ID: u64 = 1;
const RCAC_ID: u64 = 1;
const COMMISSIONER_NODE_ID: u64 = 1;

/// 2024-01-01T00:00:00Z. A clock earlier than this has not been set yet
/// (e.g. a Raspberry Pi with no RTC before NTP sync).
const SANE_CLOCK_UNIX: u64 = 1_704_067_200;

#[derive(Clone)]
pub struct Controller {
    pub inner: MatterController,
    /// Attestation roots were loaded, so `commission` can work.
    pub can_commission: bool,
}

/// Whether to load attestation roots (only needed for commissioning).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    Required,
    /// Load roots if present (the dashboard: pairing works once they are).
    IfAvailable,
    NotNeeded,
}

/// Open the controller over the persistent store, creating our fabric on
/// first use. Pairings in the store are reused, so nothing is re-paired.
pub async fn open(paths: &Paths, trust: Trust) -> Result<Controller> {
    paths.ensure_data_dir()?;
    wait_for_sane_clock().await;
    check_local_network()?;

    let store = Arc::new(FileStore::new(paths.store()));
    let mut builder = MatterController::builder(store);
    let mut can_commission = false;
    match trust {
        Trust::Required => {
            builder = builder.attestation_trust(load_trust(paths)?);
            can_commission = true;
        }
        Trust::IfAvailable => match load_trust(paths) {
            Ok(t) => {
                builder = builder.attestation_trust(t);
                can_commission = true;
            }
            Err(e) => tracing::info!("pairing unavailable: {e:#}"),
        },
        Trust::NotNeeded => {}
    }
    let controller = builder.build().await.context("opening Matter controller")?;

    let fabrics = controller.fabrics().await.context("listing fabrics")?;
    if fabrics.is_empty() {
        let now = unix_now();
        controller
            .create_fabric(FabricConfig::new(
                FABRIC_ID,
                RCAC_ID,
                COMMISSIONER_NODE_ID,
                (
                    // Backdated an hour to tolerate device clock skew.
                    MatterTime::from_unix_secs(now.saturating_sub(3600)),
                    MatterTime::NO_EXPIRY,
                ),
            ))
            .await
            .context("creating fabric")?;
        tracing::info!(store = %paths.store().display(), "created new fabric");
    }

    Ok(Controller {
        inner: controller,
        can_commission,
    })
}

fn load_trust(paths: &Paths) -> Result<AttestationTrust> {
    let (paa, cd) = (paths.paa_dir(), paths.cd_dir());
    if !paa.is_dir() || !cd.is_dir() {
        anyhow::bail!(
            "attestation roots not found in {}\nrun `mattertui fetch-roots` first",
            paths.roots_dir().display()
        );
    }
    AttestationTrust::from_dirs(&paa, &cd).with_context(|| {
        format!(
            "loading attestation roots from {} (try `mattertui fetch-roots` again)",
            paths.roots_dir().display()
        )
    })
}

/// Commissioning and CASE need a real wall clock. Wait (with a log line)
/// rather than fail if it is not set yet.
/// False on a host whose clock hasn't been set yet (Pi before NTP).
pub fn clock_is_sane() -> bool {
    unix_now() >= SANE_CLOCK_UNIX
}

async fn wait_for_sane_clock() {
    let mut warned = false;
    while unix_now() < SANE_CLOCK_UNIX {
        if !warned {
            tracing::warn!("system clock not set yet; waiting for time sync");
            warned = true;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if warned {
        tracing::info!("system clock is set");
    }
}

/// Fail fast, with a useful message, if the OS refuses LAN traffic.
///
/// macOS Local Network privacy makes every LAN send fail with
/// `EHOSTUNREACH`, which matter-controller's mDNS layer swallows, so pairing
/// otherwise fails with a misleading "not found via mDNS". We send one
/// harmless mDNS query (PTR `_matterc._udp.local`) to the multicast group and
/// check the send succeeds. On Linux this simply passes.
fn check_local_network() -> Result<()> {
    use std::net::{Ipv4Addr, UdpSocket};

    const QUERY: &[u8] = &[
        0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // header, 1 question
        8, b'_', b'm', b'a', b't', b't', b'e', b'r', b'c', //
        4, b'_', b'u', b'd', b'p', //
        5, b'l', b'o', b'c', b'a', b'l', 0, //
        0x00, 0x0C, 0x00, 0x01, // PTR, IN
    ];
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).context("binding UDP socket")?;
    match socket.send_to(QUERY, (Ipv4Addr::new(224, 0, 0, 251), 5353)) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::HostUnreachable => anyhow::bail!(
            "the OS refused to send to the local network ({e}).\n\
             On macOS: System Settings → Privacy & Security → Local Network → enable the app \
             you run mattertui from (Terminal, iTerm, Visual Studio Code, …), then restart that app.\n\
             On Linux: check the host has a multicast route (e.g. `ip route get 224.0.0.251`)."
        ),
        Err(e) => {
            tracing::warn!(error = %e, "local network check failed; continuing");
            Ok(())
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Find a paired node by its label (the config device id).
pub async fn node_by_label(controller: &MatterController, label: &str) -> Result<Option<NodeInfo>> {
    Ok(controller
        .nodes()
        .await
        .context("listing nodes")?
        .into_iter()
        .find(|n| n.label.as_deref() == Some(label)))
}

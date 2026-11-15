//! The only code that writes to devices: commissioning (plus the fabric
//! label and ICD check-in registration done straight after it) and unpairing.

use anyhow::{Context, Result, bail};
use matter_controller::{CommandPath, IcdClientType, InvokeResult, NodeInfo, ReadPath, Value};

use super::{Controller, decode, node_by_label};

const OPERATIONAL_CREDENTIALS: u32 = 0x003E;
const ATTR_CURRENT_FABRIC_INDEX: u32 = 0x0005;
const CMD_REMOVE_FABRIC: u32 = 0x0A;
const ICD_MANAGEMENT: u32 = 0x0046;
const ATTR_FEATURE_MAP: u32 = 0xFFFC;
/// IcdManagement feature bit 0: Check-In Protocol Support.
const ICD_FEATURE_CIP: u64 = 1 << 0;
/// IcdManagement feature bit 2: Long Idle Time Support.
const ICD_FEATURE_LITS: u64 = 1 << 2;

pub const FABRIC_LABEL: &str = "mattertui";

/// Device ids are used as commissioning labels, so keep them simple.
pub fn validate_device_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        bail!("invalid device id `{id}`: use only a-z, 0-9 and -");
    }
    Ok(())
}

/// Normalise a user-entered setup code. Manual codes may contain spaces or
/// dashes (`3497-011-2332`); QR payloads (`MT:...`) are passed through.
pub fn normalize_code(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("MT:") {
        return Ok(trimmed.to_string());
    }
    let digits: String = trimmed
        .chars()
        .filter(|c| !matches!(c, '-' | ' '))
        .collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        bail!("setup code must be digits (dashes/spaces allowed) or an MT: QR payload");
    }
    if digits.len() != 11 && digits.len() != 21 {
        bail!(
            "setup code has {} digits; expected 11 (or 21)",
            digits.len()
        );
    }
    if !verhoeff_valid(&digits) {
        bail!("setup code check digit is wrong; re-check the code");
    }
    Ok(digits)
}

/// Commission a device onto our fabric. The device must have an open
/// commissioning window (Home app: device → Settings → Turn On Pairing Mode).
pub async fn pair(ctl: &Controller, device_id: &str, code: &str) -> Result<NodeInfo> {
    validate_device_id(device_id)?;
    let code = normalize_code(code)?;
    if let Some(existing) = node_by_label(&ctl.inner, device_id).await? {
        bail!(
            "`{device_id}` is already paired (node 0x{:016X}); run `mattertui unpair {device_id}` first",
            existing.node_id
        );
    }

    tracing::info!(device = device_id, "commissioning…");
    let info = ctl
        .inner
        .commission(&code, Some(device_id.to_string()))
        .await
        .context("commissioning failed (is pairing mode still on in the Home app?)")?;
    tracing::info!(
        device = device_id,
        node_id = format_args!("0x{:016X}", info.node_id),
        vendor_id = ?info.vendor_id,
        product_id = ?info.product_id,
        "commissioned"
    );

    let node = ctl.inner.node(info.node_id);

    // Cosmetic but useful: shows as "mattertui" under Connected Services.
    if let Err(e) = node.update_fabric_label(FABRIC_LABEL).await {
        tracing::warn!(device = device_id, error = %e, "could not set fabric label");
    }

    // Register for ICD check-ins if the device supports them.
    match read_icd_features(&node).await {
        Ok(Some(features)) if features & ICD_FEATURE_CIP != 0 => {
            let lit = features & ICD_FEATURE_LITS != 0;
            match node
                .register_icd_client(ctl.commissioner_node_id, IcdClientType::Permanent)
                .await
            {
                Ok(_) => tracing::info!(device = device_id, lit, "registered for ICD check-ins"),
                Err(e) => {
                    tracing::warn!(device = device_id, error = %e, "ICD check-in registration failed")
                }
            }
        }
        Ok(Some(_)) => tracing::info!(device = device_id, "ICD without check-in support"),
        Ok(None) => tracing::debug!(device = device_id, "not an ICD"),
        Err(e) => tracing::warn!(device = device_id, error = %e, "could not read ICD features"),
    }

    Ok(info)
}

/// IcdManagement FeatureMap, or `None` if the device has no such cluster.
pub async fn read_icd_features(node: &matter_controller::Node) -> Result<Option<u64>> {
    let report = node
        .read(&[ReadPath::concrete(0, ICD_MANAGEMENT, ATTR_FEATURE_MAP)])
        .await?;
    Ok(decode::find(&report, 0, ICD_MANAGEMENT, ATTR_FEATURE_MAP).and_then(decode::as_u64))
}

/// How long to try reaching the device before just forgetting it locally.
const UNPAIR_DEVICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Remove our fabric from the device and forget it locally. If the device
/// is unreachable, only the local state is dropped. Returns whether the
/// device itself confirmed the removal.
pub async fn unpair(ctl: &Controller, device_id: &str) -> Result<bool> {
    let Some(info) = node_by_label(&ctl.inner, device_id).await? else {
        bail!("`{device_id}` is not paired");
    };
    let node = ctl.inner.node(info.node_id);

    let on_device = async {
        if let Err(e) = node.unregister_icd_client().await {
            tracing::debug!(device = device_id, error = %e, "ICD unregister skipped");
        }
        remove_own_fabric(&node).await
    };
    let removed = match tokio::time::timeout(UNPAIR_DEVICE_TIMEOUT, on_device).await {
        Ok(Ok(())) => {
            tracing::info!(device = device_id, "removed our fabric from the device");
            true
        }
        Ok(Err(e)) => {
            tracing::warn!(
                device = device_id,
                error = %e,
                "could not remove our fabric from the device; remove \"{FABRIC_LABEL}\" under \
                 Connected Services in the Home app"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                device = device_id,
                "device didn't answer; remove \"{FABRIC_LABEL}\" under Connected Services in the Home app"
            );
            false
        }
    };

    ctl.inner
        .forget_node(info.node_id)
        .await
        .context("forgetting node")?;
    tracing::info!(device = device_id, "forgotten locally");
    Ok(removed)
}

/// `Node::remove_fabric` deliberately refuses to remove the caller's own
/// fabric, but a fabric removing itself is legal Matter and exactly what
/// unpairing is, so invoke `RemoveFabric` directly.
async fn remove_own_fabric(node: &matter_controller::Node) -> Result<()> {
    let report = node
        .read(&[ReadPath::concrete(
            0,
            OPERATIONAL_CREDENTIALS,
            ATTR_CURRENT_FABRIC_INDEX,
        )])
        .await?;
    let index = decode::find(&report, 0, OPERATIONAL_CREDENTIALS, ATTR_CURRENT_FABRIC_INDEX)
        .and_then(decode::as_u64)
        .context("device did not report CurrentFabricIndex")?;

    let fields = Value::Structure(vec![(matter_codec::Tag::Context(0), Value::Uint(index))]);
    let path = CommandPath {
        endpoint: 0,
        cluster: OPERATIONAL_CREDENTIALS,
        command: CMD_REMOVE_FABRIC,
    };
    match node.invoke(path, fields).await? {
        InvokeResult::Data { fields, .. } => {
            // NOCResponse: field 0 is StatusCode (0 = OK).
            match decode::field(&fields, 0).and_then(decode::as_u64) {
                Some(0) => Ok(()),
                status => bail!("RemoveFabric returned status {status:?}"),
            }
        }
        other => bail!("unexpected RemoveFabric response: {other:?}"),
    }
}

/// Verhoeff checksum over a digit string (last digit is the check digit).
fn verhoeff_valid(digits: &str) -> bool {
    const D: [[u8; 10]; 10] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 2, 3, 4, 0, 6, 7, 8, 9, 5],
        [2, 3, 4, 0, 1, 7, 8, 9, 5, 6],
        [3, 4, 0, 1, 2, 8, 9, 5, 6, 7],
        [4, 0, 1, 2, 3, 9, 5, 6, 7, 8],
        [5, 9, 8, 7, 6, 0, 4, 3, 2, 1],
        [6, 5, 9, 8, 7, 1, 0, 4, 3, 2],
        [7, 6, 5, 9, 8, 2, 1, 0, 4, 3],
        [8, 7, 6, 5, 9, 3, 2, 1, 0, 4],
        [9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
    ];
    const P: [[u8; 10]; 8] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        [1, 5, 7, 6, 2, 8, 3, 0, 9, 4],
        [5, 8, 0, 3, 7, 9, 6, 1, 4, 2],
        [8, 9, 1, 6, 0, 4, 3, 5, 2, 7],
        [9, 4, 5, 3, 1, 2, 6, 8, 7, 0],
        [4, 2, 8, 6, 5, 7, 3, 9, 0, 1],
        [2, 7, 9, 3, 8, 0, 6, 4, 1, 5],
        [7, 0, 4, 6, 9, 1, 3, 2, 5, 8],
    ];
    let mut c = 0u8;
    for (i, ch) in digits.bytes().rev().enumerate() {
        let d = ch - b'0';
        c = D[c as usize][P[i % 8][d as usize] as usize];
    }
    c == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Example manual code from the Matter SDK (passcode 20202021, discr 3840).
    const VALID: &str = "34970112332";

    #[test]
    fn accepts_dashes_and_spaces() {
        assert_eq!(normalize_code("3497-011-2332").unwrap(), VALID);
        assert_eq!(normalize_code(" 3497 011 2332 ").unwrap(), VALID);
    }

    #[test]
    fn rejects_bad_check_digit() {
        assert!(normalize_code("34970112333").is_err());
    }

    #[test]
    fn rejects_wrong_length_and_letters() {
        assert!(normalize_code("1234").is_err());
        assert!(normalize_code("3497a112332").is_err());
    }

    #[test]
    fn passes_qr_through() {
        assert_eq!(
            normalize_code("MT:Y.K90AFN00KA0648G00").unwrap(),
            "MT:Y.K90AFN00KA0648G00"
        );
    }

    #[test]
    fn device_ids() {
        assert!(validate_device_id("lounge-lamp-2").is_ok());
        assert!(validate_device_id("Lounge").is_err());
        assert!(validate_device_id("").is_err());
    }
}

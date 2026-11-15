//! Work out what to subscribe to for a configured device: pick the endpoint
//! for its kind and keep only attributes the device actually implements.

use anyhow::{Result, bail};
use matter_controller::ReadPath;

use super::decode::{self, Report};
use super::readonly::ReadOnlyNode;
use crate::config::DeviceKind;
use crate::state::{DeviceInfo, Transport};

pub const BASIC_INFORMATION: u32 = 0x0028;
pub const DESCRIPTOR: u32 = 0x001D;
pub const NETWORK_COMMISSIONING: u32 = 0x0031;
pub const ON_OFF: u32 = 0x0006;
pub const THERMOSTAT: u32 = 0x0201;
pub const TEMPERATURE: u32 = 0x0402;
pub const HUMIDITY: u32 = 0x0405;
pub const POWER_SOURCE: u32 = 0x002F;
pub const FEATURE_MAP: u32 = 0xFFFC;
pub const ATTRIBUTE_LIST: u32 = 0xFFFB;

const DEVICE_TYPE_LIST: u32 = 0x0000;
const SERVER_LIST: u32 = 0x0001;

pub struct Discovery {
    pub info: DeviceInfo,
    /// Concrete attribute paths to subscribe to.
    pub paths: Vec<ReadPath>,
}

pub async fn discover(
    node: &ReadOnlyNode,
    kind: DeviceKind,
    endpoint_override: Option<u16>,
) -> Result<Discovery> {
    let report = node
        .read(&[
            ReadPath::concrete(0, BASIC_INFORMATION, 0x0001), // VendorName
            ReadPath::concrete(0, BASIC_INFORMATION, 0x0003), // ProductName
            ReadPath::concrete(0, BASIC_INFORMATION, 0x000A), // SoftwareVersionString
            ReadPath::new(None, Some(DESCRIPTOR), Some(DEVICE_TYPE_LIST)),
            ReadPath::new(None, Some(DESCRIPTOR), Some(SERVER_LIST)),
            ReadPath::new(None, Some(NETWORK_COMMISSIONING), Some(FEATURE_MAP)),
        ])
        .await?;

    let endpoints = endpoints(&report);
    let endpoint = match endpoint_override {
        Some(ep) => ep,
        None => choose_endpoint(&endpoints, kind)?,
    };

    // Which attributes exist on the clusters we care about.
    let main_cluster = match kind {
        DeviceKind::Plug => ON_OFF,
        DeviceKind::Thermostat => THERMOSTAT,
        DeviceKind::TemperatureSensor => TEMPERATURE,
    };
    let mut wanted: Vec<(u16, u32, &[u32])> = vec![(
        endpoint,
        main_cluster,
        match kind {
            DeviceKind::Plug => &[0x0000],
            // LocalTemperature, Cooling/HeatingSetpoint, SystemMode, RunningState
            DeviceKind::Thermostat => &[0x0000, 0x0011, 0x0012, 0x001C, 0x0029],
            DeviceKind::TemperatureSensor => &[0x0000],
        },
    )];
    if kind == DeviceKind::TemperatureSensor
        && let Some(ep) = first_with_server(&endpoints, HUMIDITY, endpoint)
    {
        wanted.push((ep, HUMIDITY, &[0x0000]));
    }
    if let Some(ep) = first_with_server(&endpoints, POWER_SOURCE, endpoint) {
        wanted.push((ep, POWER_SOURCE, &[0x000C])); // BatPercentRemaining
    }

    let lists = node
        .read(
            &wanted
                .iter()
                .map(|(ep, c, _)| ReadPath::concrete(*ep, *c, ATTRIBUTE_LIST))
                .collect::<Vec<_>>(),
        )
        .await?;
    let mut paths = Vec::new();
    for (ep, cluster, attrs) in &wanted {
        let present: Vec<u64> = decode::find(&lists, *ep, *cluster, ATTRIBUTE_LIST)
            .map(decode::elements)
            .unwrap_or_default()
            .into_iter()
            .filter_map(decode::as_u64)
            .collect();
        for &a in *attrs {
            // If the device didn't return an AttributeList, ask anyway.
            if present.is_empty() || present.contains(&u64::from(a)) {
                paths.push(ReadPath::concrete(*ep, *cluster, a));
            }
        }
    }

    let s = |a| decode::find(&report, 0, BASIC_INFORMATION, a).and_then(decode::as_str).map(str::to_owned);
    let info = DeviceInfo {
        node_id: node.node_id(),
        vendor: s(0x0001),
        product: s(0x0003),
        firmware: s(0x000A),
        transport: transport(&report),
        endpoint,
    };
    Ok(Discovery { info, paths })
}

struct Endpoint {
    id: u16,
    device_types: Vec<u64>,
    servers: Vec<u64>,
}

fn endpoints(report: &Report) -> Vec<Endpoint> {
    let mut ids: Vec<u16> = report
        .iter()
        .filter(|(p, _)| p.cluster == DESCRIPTOR)
        .map(|(p, _)| p.endpoint)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| Endpoint {
            id,
            device_types: decode::find(report, id, DESCRIPTOR, DEVICE_TYPE_LIST)
                .map(decode::elements)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|e| decode::field(e, 0).and_then(decode::as_u64))
                .collect(),
            servers: decode::find(report, id, DESCRIPTOR, SERVER_LIST)
                .map(decode::elements)
                .unwrap_or_default()
                .into_iter()
                .filter_map(decode::as_u64)
                .collect(),
        })
        .collect()
}

fn choose_endpoint(endpoints: &[Endpoint], kind: DeviceKind) -> Result<u16> {
    let (types, cluster): (&[u64], u32) = match kind {
        // On/Off Plug-in Unit, Dimmable Plug-in Unit, On/Off Light, Dimmable Light
        DeviceKind::Plug => (&[0x010A, 0x010B, 0x0100, 0x0101], ON_OFF),
        DeviceKind::Thermostat => (&[0x0301], THERMOSTAT),
        DeviceKind::TemperatureSensor => (&[0x0302], TEMPERATURE),
    };
    endpoints
        .iter()
        .find(|e| e.device_types.iter().any(|t| types.contains(t)))
        .or_else(|| {
            endpoints
                .iter()
                .find(|e| e.id != 0 && e.servers.contains(&u64::from(cluster)))
        })
        .map(|e| e.id)
        .map_or_else(
            || bail!("no endpoint looks like a {kind:?}; set `endpoint` in the config (see `mattertui nodes --probe`)"),
            Ok,
        )
}

/// Prefer `preferred` if it has the cluster, else the first endpoint that does.
fn first_with_server(endpoints: &[Endpoint], cluster: u32, preferred: u16) -> Option<u16> {
    let has = |e: &&Endpoint| e.servers.contains(&u64::from(cluster));
    endpoints
        .iter()
        .filter(has)
        .find(|e| e.id == preferred)
        .or_else(|| endpoints.iter().find(has))
        .map(|e| e.id)
}

fn transport(report: &Report) -> Transport {
    let f = report
        .iter()
        .filter(|(p, _)| p.cluster == NETWORK_COMMISSIONING && p.attribute == FEATURE_MAP)
        .filter_map(|(_, v)| decode::as_u64(v))
        .fold(0, |a, b| a | b);
    if f & 1 != 0 {
        Transport::WiFi
    } else if f & 2 != 0 {
        Transport::Thread
    } else if f & 4 != 0 {
        Transport::Ethernet
    } else {
        Transport::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: u16, device_types: &[u64], servers: &[u64]) -> Endpoint {
        Endpoint {
            id,
            device_types: device_types.to_vec(),
            servers: servers.to_vec(),
        }
    }

    #[test]
    fn picks_endpoint_by_device_type() {
        let eps = [ep(0, &[0x16], &[0x28]), ep(1, &[0x010A], &[0x06])];
        assert_eq!(choose_endpoint(&eps, DeviceKind::Plug).unwrap(), 1);
        assert!(choose_endpoint(&eps, DeviceKind::Thermostat).is_err());
    }

    #[test]
    fn falls_back_to_server_cluster() {
        let eps = [ep(0, &[0x16], &[]), ep(2, &[0xFFFF], &[0x0402])];
        assert_eq!(choose_endpoint(&eps, DeviceKind::TemperatureSensor).unwrap(), 2);
    }

    #[test]
    fn prefers_same_endpoint_for_extra_clusters() {
        let eps = [ep(0, &[], &[0x2F]), ep(1, &[], &[0x2F, 0x405])];
        assert_eq!(first_with_server(&eps, POWER_SOURCE, 1), Some(1));
        assert_eq!(first_with_server(&eps, POWER_SOURCE, 3), Some(0));
        assert_eq!(first_with_server(&eps, 0x9999, 1), None);
    }
}

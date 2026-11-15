//! `mattertui nodes --probe`: read what each paired device exposes. Used to
//! check pairing worked and to write the config (endpoints, kinds, ICD type).

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::Result;
use matter_controller::{Node, NodeInfo, ReadPath};

use super::decode::{self, Report};

const BASIC_INFORMATION: u32 = 0x0028;
const DESCRIPTOR: u32 = 0x001D;
const NETWORK_COMMISSIONING: u32 = 0x0031;
const ICD_MANAGEMENT: u32 = 0x0046;
const ON_OFF: u32 = 0x0006;
const THERMOSTAT: u32 = 0x0201;
const TEMPERATURE: u32 = 0x0402;
const HUMIDITY: u32 = 0x0405;
const POWER_SOURCE: u32 = 0x002F;
const FEATURE_MAP: u32 = 0xFFFC;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Probe one node and return a human-readable multi-line summary.
pub async fn probe(node: &Node, info: &NodeInfo) -> String {
    match tokio::time::timeout(PROBE_TIMEOUT, probe_inner(node)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => format!("    unreachable: {e:#}\n"),
        Err(_) => format!(
            "    no answer within {}s (asleep or unreachable?) node 0x{:016X}\n",
            PROBE_TIMEOUT.as_secs(),
            info.node_id
        ),
    }
}

async fn probe_inner(node: &Node) -> Result<String> {
    let report = node
        .read(&[
            ReadPath::concrete(0, BASIC_INFORMATION, 0x0001), // VendorName
            ReadPath::concrete(0, BASIC_INFORMATION, 0x0003), // ProductName
            ReadPath::concrete(0, BASIC_INFORMATION, 0x000A), // SoftwareVersionString
            ReadPath::new(None, Some(DESCRIPTOR), Some(0x0000)), // DeviceTypeList
            ReadPath::new(None, Some(DESCRIPTOR), Some(0x0001)), // ServerList
            ReadPath::new(None, Some(NETWORK_COMMISSIONING), Some(FEATURE_MAP)),
            ReadPath::cluster(0, ICD_MANAGEMENT),
        ])
        .await?;

    let mut out = String::new();
    let s = |a| decode::find(&report, 0, BASIC_INFORMATION, a).and_then(decode::as_str);
    let _ = writeln!(
        out,
        "    {} {} (fw {})",
        s(0x0001).unwrap_or("?"),
        s(0x0003).unwrap_or("?"),
        s(0x000A).unwrap_or("?")
    );

    let _ = writeln!(out, "    transport: {}", transport(&report));
    let _ = writeln!(out, "    icd: {}", icd_summary(&report));

    // Endpoints with their device types and server clusters, then the
    // values we care about on each.
    let mut endpoints: Vec<u16> = report
        .iter()
        .filter(|(p, _)| p.cluster == DESCRIPTOR && p.attribute == 0x0000)
        .map(|(p, _)| p.endpoint)
        .collect();
    endpoints.sort_unstable();
    endpoints.dedup();

    let mut value_paths = Vec::new();
    for &ep in &endpoints {
        let types: Vec<u64> = decode::find(&report, ep, DESCRIPTOR, 0x0000)
            .map(decode::elements)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| decode::field(e, 0).and_then(decode::as_u64))
            .collect();
        let servers: Vec<u64> = decode::find(&report, ep, DESCRIPTOR, 0x0001)
            .map(decode::elements)
            .unwrap_or_default()
            .into_iter()
            .filter_map(decode::as_u64)
            .collect();
        let names: Vec<String> = types.iter().map(|t| device_type_name(*t)).collect();
        let _ = writeln!(out, "    endpoint {ep}: {}", names.join(", "));

        for (cluster, attr) in [
            (ON_OFF, 0x0000),
            (THERMOSTAT, 0x0000),
            (THERMOSTAT, 0x0012),
            (THERMOSTAT, 0x001C),
            (TEMPERATURE, 0x0000),
            (HUMIDITY, 0x0000),
            (POWER_SOURCE, 0x000C),
        ] {
            if servers.contains(&u64::from(cluster)) {
                value_paths.push(ReadPath::concrete(ep, cluster, attr));
            }
        }
    }

    if !value_paths.is_empty() {
        let values = node.read(&value_paths).await?;
        for (path, value) in &values {
            let rendered = match (path.cluster, path.attribute) {
                (ON_OFF, 0) => decode::as_bool(value).map(|b| if b { "on" } else { "off" }.into()),
                (THERMOSTAT, 0) => fmt_temp(value, "temperature"),
                (THERMOSTAT, 0x12) => fmt_temp(value, "heating setpoint"),
                (THERMOSTAT, 0x1C) => decode::as_u64(value).map(|m| format!("mode {}", system_mode(m))),
                (TEMPERATURE, 0) => fmt_temp(value, "temperature"),
                (HUMIDITY, 0) => decode::as_u64(value).map(|h| format!("humidity {:.1}%", h as f64 / 100.0)),
                (POWER_SOURCE, 0x0C) => decode::as_u64(value).map(|b| format!("battery {}%", b / 2)),
                _ => None,
            };
            let _ = writeln!(
                out,
                "      ep{} {}",
                path.endpoint,
                rendered.unwrap_or_else(|| format!("{:04X}/{:04X} = {value:?}", path.cluster, path.attribute))
            );
        }
    }
    Ok(out)
}

fn fmt_temp(v: &matter_controller::Value, what: &str) -> Option<String> {
    Some(match decode::centi_celsius(v) {
        Some(c) => format!("{what} {c:.2}°C"),
        None => format!("{what} unknown"),
    })
}

fn transport(report: &Report) -> &'static str {
    let features: u64 = report
        .iter()
        .filter(|(p, _)| p.cluster == NETWORK_COMMISSIONING && p.attribute == FEATURE_MAP)
        .filter_map(|(_, v)| decode::as_u64(v))
        .fold(0, |a, b| a | b);
    match (features & 1 != 0, features & 2 != 0, features & 4 != 0) {
        (true, _, _) => "Wi-Fi",
        (_, true, _) => "Thread",
        (_, _, true) => "Ethernet",
        _ => "unknown",
    }
}

fn icd_summary(report: &Report) -> String {
    let Some(features) =
        decode::find(report, 0, ICD_MANAGEMENT, FEATURE_MAP).and_then(decode::as_u64)
    else {
        return "no (always on)".into();
    };
    let attr = |a| decode::find(report, 0, ICD_MANAGEMENT, a).and_then(decode::as_u64);
    let kind = if features & 0b100 != 0 { "LIT" } else { "SIT" };
    let cip = if features & 0b1 != 0 { "check-in supported" } else { "no check-in" };
    format!(
        "{kind}, {cip}, idle {}s, active {}ms, registered clients {}",
        attr(0x0000).map_or("?".into(), |v| v.to_string()), // IdleModeDuration (s)
        attr(0x0001).map_or("?".into(), |v| v.to_string()), // ActiveModeDuration (ms)
        decode::find(report, 0, ICD_MANAGEMENT, 0x0003)      // RegisteredClients
            .map(|v| decode::elements(v).len().to_string())
            .unwrap_or_else(|| "?".into()),
    )
}

fn device_type_name(t: u64) -> String {
    let name = match t {
        0x0011 => "Power Source",
        0x0012 => "OTA Requestor",
        0x0013 => "Bridged Node",
        0x0016 => "Root Node",
        0x000E => "Aggregator",
        0x0100 => "On/Off Light",
        0x0101 => "Dimmable Light",
        0x010A => "On/Off Plug-in Unit",
        0x010B => "Dimmable Plug-in Unit",
        0x0301 => "Thermostat",
        0x0302 => "Temperature Sensor",
        0x0307 => "Humidity Sensor",
        0x0510 => "Electrical Sensor",
        _ => return format!("0x{t:04X}"),
    };
    format!("{name} (0x{t:04X})")
}

fn system_mode(m: u64) -> &'static str {
    match m {
        0 => "off",
        1 => "auto",
        3 => "cool",
        4 => "heat",
        5 => "emergency heat",
        6 => "precooling",
        7 => "fan only",
        8 => "dry",
        9 => "sleep",
        _ => "?",
    }
}

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
const BOOLEAN_STATE: u32 = 0x0045;
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
    let mut thermostat_eps = Vec::new();
    let mut power_eps = Vec::new();
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
        let clusters: Vec<String> = servers.iter().map(|c| cluster_name(*c)).collect();
        let _ = writeln!(out, "      clusters: {}", clusters.join(", "));
        if servers.contains(&u64::from(THERMOSTAT)) {
            thermostat_eps.push(ep);
        }
        if servers.contains(&u64::from(POWER_SOURCE)) {
            power_eps.push(ep);
        }

        for (cluster, attr) in [
            (ON_OFF, 0x0000),
            (THERMOSTAT, 0x0000),
            (THERMOSTAT, 0x0012),
            (THERMOSTAT, 0x001C),
            (TEMPERATURE, 0x0000),
            (HUMIDITY, 0x0000),
            (POWER_SOURCE, 0x000C),
            (BOOLEAN_STATE, 0x0000),
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
                (BOOLEAN_STATE, 0) => decode::as_bool(value).map(|b| format!("state {b} (leak sensor: {})", if b { "LEAK" } else { "dry" })),
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
    // Battery reporting varies too (percentage vs charge level).
    for ep in power_eps {
        let attrs = node.read(&[ReadPath::cluster(ep, POWER_SOURCE)]).await?;
        let _ = writeln!(out, "    power source attributes (endpoint {ep}):");
        let mut attrs: Vec<_> = attrs.iter().filter(|(p, _)| p.endpoint == ep && p.cluster == POWER_SOURCE).collect();
        attrs.sort_by_key(|(p, _)| p.attribute);
        for (p, v) in attrs {
            let name = match p.attribute {
                0x0000 => "Status",
                0x0001 => "Order",
                0x0002 => "Description",
                0x000B => "BatVoltage (mV)",
                0x000C => "BatPercentRemaining (½%)",
                0x000D => "BatTimeRemaining (s)",
                0x000E => "BatChargeLevel (0 ok,1 warn,2 crit)",
                0x000F => "BatReplacementNeeded",
                0x0010 => "BatReplaceability",
                0x0011 => "BatPresent",
                0x0013 => "BatReplacementDescription",
                0x0014 => "BatCommonDesignation",
                0x0017 => "BatQuantity",
                0x001F => "EndpointList",
                0xFFF8..=0xFFFD => continue,
                _ => "?",
            };
            let s = format!("{v:?}");
            let s = if s.len() > 80 { format!("{}…", &s[..80]) } else { s };
            let _ = writeln!(out, "      0x{:04X} {name:<36} {s}", p.attribute);
        }
    }

    // Thermostats vary a lot between devices: dump every attribute.
    for ep in thermostat_eps {
        let attrs = node.read(&[ReadPath::cluster(ep, THERMOSTAT)]).await?;
        let _ = writeln!(out, "    thermostat attributes (endpoint {ep}):");
        let mut attrs: Vec<_> = attrs.iter().filter(|(p, _)| p.endpoint == ep && p.cluster == THERMOSTAT).collect();
        attrs.sort_by_key(|(p, _)| p.attribute);
        for (p, v) in attrs {
            let _ = writeln!(
                out,
                "      0x{:04X} {:<32} {}",
                p.attribute,
                thermostat_attr_name(p.attribute),
                thermostat_value(p.attribute, v)
            );
        }
    }
    Ok(out)
}

fn thermostat_attr_name(a: u32) -> &'static str {
    match a {
        0x0000 => "LocalTemperature",
        0x0001 => "OutdoorTemperature",
        0x0002 => "Occupancy",
        0x0003 => "AbsMinHeatSetpointLimit",
        0x0004 => "AbsMaxHeatSetpointLimit",
        0x0005 => "AbsMinCoolSetpointLimit",
        0x0006 => "AbsMaxCoolSetpointLimit",
        0x0007 => "PICoolingDemand",
        0x0008 => "PIHeatingDemand",
        0x0010 => "LocalTemperatureCalibration",
        0x0011 => "OccupiedCoolingSetpoint",
        0x0012 => "OccupiedHeatingSetpoint",
        0x0013 => "UnoccupiedCoolingSetpoint",
        0x0014 => "UnoccupiedHeatingSetpoint",
        0x0015 => "MinHeatSetpointLimit",
        0x0016 => "MaxHeatSetpointLimit",
        0x0017 => "MinCoolSetpointLimit",
        0x0018 => "MaxCoolSetpointLimit",
        0x0019 => "MinSetpointDeadBand",
        0x001A => "RemoteSensing",
        0x001B => "ControlSequenceOfOperation",
        0x001C => "SystemMode",
        0x001E => "ThermostatRunningMode",
        0x0020 => "StartOfWeek",
        0x0021 => "NumberOfWeeklyTransitions",
        0x0022 => "NumberOfDailyTransitions",
        0x0023 => "TemperatureSetpointHold",
        0x0024 => "TemperatureSetpointHoldDuration",
        0x0025 => "ThermostatProgrammingOperationMode",
        0x0029 => "ThermostatRunningState",
        0x0030 => "SetpointChangeSource",
        0x0031 => "SetpointChangeAmount",
        0x0032 => "SetpointChangeSourceTimestamp",
        0x0034 => "OccupiedSetback",
        0x0040 => "EmergencyHeatDelta",
        0x0048 => "PresetTypes",
        0x0049 => "ScheduleTypes",
        0x004A => "NumberOfPresets",
        0x004E => "ActivePresetHandle",
        0x0050 => "Presets",
        0x0051 => "Schedules",
        0x0052 => "SetpointHoldExpiryTimestamp",
        0xFFF8 => "GeneratedCommandList",
        0xFFF9 => "AcceptedCommandList",
        0xFFFB => "AttributeList",
        0xFFFC => "FeatureMap",
        0xFFFD => "ClusterRevision",
        _ => "?",
    }
}

fn thermostat_value(a: u32, v: &matter_controller::Value) -> String {
    let temp = |v| fmt_temp(v, "").map_or_else(|| format!("{v:?}"), |s| s.trim().to_string());
    match a {
        0x0000 | 0x0001 | 0x0003..=0x0006 | 0x0011..=0x0018 => temp(v),
        0x0007 | 0x0008 => decode::as_u64(v).map_or_else(|| format!("{v:?}"), |p| format!("{p}%")),
        0x001C => decode::as_u64(v).map_or_else(|| format!("{v:?}"), |m| format!("{m} ({})", system_mode(m))),
        0x001B => decode::as_u64(v).map_or_else(
            || format!("{v:?}"),
            |s| {
                let name = match s {
                    0 => "cooling only",
                    1 => "cooling with reheat",
                    2 => "heating only",
                    3 => "heating with reheat",
                    4 => "cooling and heating",
                    5 => "cooling and heating with reheat",
                    _ => "?",
                };
                format!("{s} ({name})")
            },
        ),
        0x0029 => decode::as_u64(v).map_or_else(|| format!("{v:?}"), |b| format!("0b{b:07b} (bit0 heat, bit1 cool, bit2 fan)")),
        0xFFF8 | 0xFFF9 | 0xFFFB => {
            let ids: Vec<String> = decode::elements(v).into_iter().filter_map(decode::as_u64).map(|i| format!("{i:#06x}")).collect();
            format!("[{}]", ids.join(" "))
        }
        _ => {
            let s = format!("{v:?}");
            if s.len() > 100 { format!("{}…", &s[..100]) } else { s }
        }
    }
}

fn cluster_name(c: u64) -> String {
    let name = match c {
        0x0003 => "Identify",
        0x0004 => "Groups",
        0x0006 => "OnOff",
        0x001D => "Descriptor",
        0x001E => "Binding",
        0x001F => "AccessControl",
        0x0028 => "BasicInformation",
        0x0029 => "OtaProvider",
        0x002A => "OtaRequestor",
        0x002F => "PowerSource",
        0x0030 => "GeneralCommissioning",
        0x0031 => "NetworkCommissioning",
        0x0032 => "DiagnosticLogs",
        0x0033 => "GeneralDiagnostics",
        0x0035 => "ThreadNetworkDiagnostics",
        0x0036 => "WiFiNetworkDiagnostics",
        0x003C => "AdministratorCommissioning",
        0x003E => "OperationalCredentials",
        0x003F => "GroupKeyManagement",
        0x0040 => "FixedLabel",
        0x0045 => "BooleanState",
        0x0080 => "BooleanStateConfiguration",
        0x0041 => "UserLabel",
        0x0046 => "IcdManagement",
        0x0090 => "ElectricalPowerMeasurement",
        0x0091 => "ElectricalEnergyMeasurement",
        0x0201 => "Thermostat",
        0x0202 => "FanControl",
        0x0204 => "ThermostatUserInterfaceConfiguration",
        0x0402 => "TemperatureMeasurement",
        0x0405 => "RelativeHumidityMeasurement",
        0x0406 => "OccupancySensing",
        _ => return format!("{c:#06x}"),
    };
    name.to_string()
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
        0x0043 => "Water Leak Detector",
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

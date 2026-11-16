//! Helpers for pulling typed values out of `matter_codec::Value`.

use matter_controller::{AttributePath, Value};

use crate::state::Metric;

pub type Report = [(AttributePath, Value)];

/// Find the value for a concrete attribute in a read report.
pub fn find(report: &Report, endpoint: u16, cluster: u32, attr: u32) -> Option<&Value> {
    report
        .iter()
        .find(|(p, _)| p.endpoint == endpoint && p.cluster == cluster && p.attribute == attr)
        .map(|(_, v)| v)
}

pub fn as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Uint(u) => Some(*u),
        Value::Int(i) => u64::try_from(*i).ok(),
        _ => None,
    }
}

pub fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Uint(u) => i64::try_from(*u).ok(),
        _ => None,
    }
}

pub fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

pub fn as_str(v: &Value) -> Option<&str> {
    match v {
        Value::Utf8(s) => Some(s),
        _ => None,
    }
}

/// Elements of an array/list attribute, ignoring tags.
pub fn elements(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(items) => items.iter().collect(),
        Value::List(items) | Value::Structure(items) => items.iter().map(|(_, v)| v).collect(),
        _ => Vec::new(),
    }
}

/// A context-tagged field of a structure.
pub fn field(v: &Value, tag: u8) -> Option<&Value> {
    match v {
        Value::Structure(items) | Value::List(items) => items
            .iter()
            .find(|(t, _)| *t == matter_codec::Tag::Context(tag))
            .map(|(_, v)| v),
        _ => None,
    }
}

/// Matter temperatures are int16 in 0.01 °C; null means "unknown".
pub fn centi_celsius(v: &Value) -> Option<f64> {
    as_i64(v).map(|c| c as f64 / 100.0)
}

/// Map a reported attribute to a metric. `None` if we don't track it.
/// The inner `Option` is `None` when the device reports null.
pub fn reading(cluster: u32, attribute: u32, v: &Value) -> Option<(Metric, Option<f64>)> {
    use super::discover::{BOOLEAN_STATE, HUMIDITY, ON_OFF, POWER_SOURCE, TEMPERATURE, THERMOSTAT};
    let num = |v: &Value| as_u64(v).map(|n| n as f64);
    let flag = |v: &Value| as_bool(v).map(|b| if b { 1.0 } else { 0.0 });
    Some(match (cluster, attribute) {
        (ON_OFF, 0x0000) => (Metric::On, flag(v)),
        (BOOLEAN_STATE, 0x0000) => (Metric::Leak, flag(v)),
        (THERMOSTAT, 0x0000) | (TEMPERATURE, 0x0000) => (Metric::Temperature, centi_celsius(v)),
        (THERMOSTAT, 0x0011) => (Metric::CoolSetpoint, centi_celsius(v)),
        (THERMOSTAT, 0x0012) => (Metric::HeatSetpoint, centi_celsius(v)),
        (THERMOSTAT, 0x001C) => (Metric::Mode, num(v)),
        (THERMOSTAT, 0x0029) => (Metric::Running, num(v)),
        (THERMOSTAT, 0x0008) => (Metric::HeatDemand, num(v)),
        (POWER_SOURCE, 0x000E) => (Metric::BatteryLevel, num(v)),
        (HUMIDITY, 0x0000) => (Metric::Humidity, num(v).map(|h| h / 100.0)),
        // BatPercentRemaining is in half-percent units.
        (POWER_SOURCE, 0x000C) => (Metric::Battery, num(v).map(|b| b / 2.0)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use matter_codec::Tag;

    #[test]
    fn maps_reports_to_metrics() {
        assert_eq!(reading(0x0006, 0, &Value::Bool(true)), Some((Metric::On, Some(1.0))));
        assert_eq!(
            reading(0x0402, 0, &Value::Int(1875)),
            Some((Metric::Temperature, Some(18.75)))
        );
        assert_eq!(reading(0x0402, 0, &Value::Null), Some((Metric::Temperature, None)));
        assert_eq!(reading(0x0405, 0, &Value::Uint(4520)), Some((Metric::Humidity, Some(45.2))));
        assert_eq!(reading(0x002F, 0x0C, &Value::Uint(200)), Some((Metric::Battery, Some(100.0))));
        assert_eq!(reading(0x0201, 0x0008, &Value::Uint(45)), Some((Metric::HeatDemand, Some(45.0))));
        assert_eq!(reading(0x002F, 0x0E, &Value::Uint(1)), Some((Metric::BatteryLevel, Some(1.0))));
        assert_eq!(reading(0x0045, 0, &Value::Bool(true)), Some((Metric::Leak, Some(1.0))));
        assert_eq!(reading(0x0045, 0, &Value::Bool(false)), Some((Metric::Leak, Some(0.0))));
        assert_eq!(reading(0x0006, 0x4000, &Value::Bool(true)), None);
    }

    #[test]
    fn temperature_decodes_and_null_is_none() {
        assert_eq!(centi_celsius(&Value::Int(2154)), Some(21.54));
        assert_eq!(centi_celsius(&Value::Int(-150)), Some(-1.5));
        assert_eq!(centi_celsius(&Value::Null), None);
    }

    #[test]
    fn struct_fields_by_context_tag() {
        let v = Value::Structure(vec![
            (Tag::Context(0), Value::Uint(0x0302)),
            (Tag::Context(1), Value::Uint(2)),
        ]);
        assert_eq!(field(&v, 0).and_then(as_u64), Some(0x0302));
        assert_eq!(field(&v, 1).and_then(as_u64), Some(2));
        assert!(field(&v, 2).is_none());
    }
}

//! One snapshot per successful NVMe SMART scan. These readings are independent
//! of the since-boot activity counters collected by Disk & Memory.

use serde_json::{Map, Value};

use crate::collector::{Collector, CollectorInfo, DayInput, Kind, MetricDef, MetricValue, Sink};
use crate::time::Timestamp;

pub const NAME: &str = "drive_health";
pub type ProbeResult = Result<Vec<u8>, Unavailable>;
pub type Probe = fn(&Sink) -> ProbeResult;

#[derive(Debug, Clone, Copy)]
pub enum Unavailable {
    HelperMissing,
    NoInternalNvme,
    HealthData,
    TimedOut,
}

impl Unavailable {
    fn status(self) -> &'static str {
        match self {
            Self::HelperMissing => "helper_missing",
            Self::NoInternalNvme => "no_internal_nvme",
            Self::HealthData => "health_unavailable",
            Self::TimedOut => "timed_out",
        }
    }
}

pub struct DriveHealth;

impl Collector for DriveHealth {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "Drive Health",
            description: "Daily SSD lifetime reads and writes, endurance used, temperature, and power-on hours. Requires smartctl.",
            permissions: "None (macOS)",
            cost: "One short daily scan",
            default_on: false,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Scanned
    }

    fn start(&mut self, _out: Sink) {}
    fn stop(&mut self) {}

    fn scanner(&self) -> Option<Box<dyn Collector>> {
        Some(Box::new(Self))
    }

    fn scan(&mut self, _since: Option<Timestamp>, out: Sink) {
        if out.cancelled() {
            return;
        }
        let reading = out
            .read_drive_health()
            .and_then(|json| parse(&json).ok_or(Unavailable::HealthData));
        if out.cancelled() {
            return;
        }
        let status = match reading {
            Ok(fields) => {
                let now = out.now();
                out.item(now.utc_ms.to_string(), now, true, fields.to_string());
                "ok"
            }
            Err(reason) => reason.status(),
        };
        out.count(status, 1);
        out.save_scan_state(serde_json::json!(status).to_string());
    }

    fn metrics(&self) -> Vec<MetricDef> {
        Vec::new()
    }

    fn rollup(&self, _day: &DayInput<'_>) -> Vec<MetricValue> {
        Vec::new()
    }
}

/// Allowlist the stored fields. A missing or malformed field in an otherwise
/// usable reading stays absent, and a zero NVMe data-unit count is unreported.
fn parse(stdout: &[u8]) -> Option<Value> {
    let json: Value = serde_json::from_slice(stdout).ok()?;
    let log = json.get("nvme_smart_health_information_log")?.as_object()?;
    let bytes = |key| {
        log.get(key)?
            .as_u64()
            .filter(|&units| units != 0)?
            .checked_mul(512_000)
    };
    let fields = [
        ("host_bytes_read", bytes("data_units_read").map(Value::from)),
        (
            "host_bytes_written",
            bytes("data_units_written").map(Value::from),
        ),
        (
            "endurance_used_percent",
            log.get("percentage_used")
                .and_then(Value::as_u64)
                .filter(|&pct| pct <= 255)
                .map(Value::from),
        ),
        (
            "temperature_c",
            log.get("temperature")
                .and_then(Value::as_i64)
                .filter(|c| (-272..=65_262).contains(c))
                .map(Value::from),
        ),
        (
            "power_on_hours",
            log.get("power_on_hours")
                .and_then(Value::as_u64)
                .map(Value::from),
        ),
    ];
    let fields: Map<String, Value> = fields
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_owned(), value?)))
        .collect();
    (!fields.is_empty()).then_some(Value::Object(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_response_converts_units_and_keeps_only_health_fields() {
        assert_eq!(
            parse(include_bytes!("drive_health/full.json")),
            Some(json!({
                "host_bytes_read": 51_406_421_504_000u64,
                "host_bytes_written": 35_192_018_432_000u64,
                "endurance_used_percent": 1,
                "temperature_c": 33,
                "power_on_hours": 594
            }))
        );
    }

    #[test]
    fn partial_response_keeps_valid_fields_including_zero_wear() {
        assert_eq!(
            parse(include_bytes!("drive_health/partial.json")),
            Some(json!({
                "endurance_used_percent": 0,
                "temperature_c": -2
            }))
        );
    }

    #[test]
    fn invalid_response_has_no_snapshot() {
        assert_eq!(parse(include_bytes!("drive_health/invalid.json")), None);
        assert_eq!(parse(b"{"), None);
        assert_eq!(parse(b"{}"), None);
    }
}

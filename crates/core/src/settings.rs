//! Tracking settings, stored as key/value rows (JSON values) in `settings`.
//!
//! | key                                  | value (JSON)                            | default |
//! |--------------------------------------|-----------------------------------------|---------|
//! | `collector.<name>.enabled`           | `true` / `false`                        | collector's `default_on` |
//! | `idle_threshold_secs`                | integer, 60..=86400                     | 180     |
//! | `active_without_input.apps`          | array of bundle ids                     | `[]`    |
//! | `active_without_input.timeout_secs`  | integer, 60..=86400                     | 1800    |
//! | `exclude.apps`                       | array of bundle ids                     | `[]`    |
//! | `pause`                              | `"off"`, `"indefinite"` or `{"until_ms": <utc ms>}` | `"off"` |
//!
//! Keys starting with `internal.` are bookkeeping owned by the store and are
//! not user settings.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Value, json};

use crate::time::{MS_PER_HOUR, MS_PER_MIN, Timestamp, local_midnight_ms};

pub const KEY_IDLE_THRESHOLD: &str = "idle_threshold_secs";
pub const KEY_AWI_APPS: &str = "active_without_input.apps";
pub const KEY_AWI_TIMEOUT: &str = "active_without_input.timeout_secs";
pub const KEY_EXCLUDE: &str = "exclude.apps";
pub const KEY_PAUSE: &str = "pause";
pub const INTERNAL_PREFIX: &str = "internal.";

pub const DEFAULT_IDLE_THRESHOLD_SECS: u32 = 180;
pub const DEFAULT_AWI_TIMEOUT_SECS: u32 = 1800;
const MIN_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 86_400;

pub fn collector_enabled_key(collector: &str) -> String {
    format!("collector.{collector}.enabled")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pause {
    #[default]
    Off,
    /// Paused until this UTC millisecond.
    Until(i64),
    Indefinite,
}

impl Pause {
    pub fn is_active(&self, now: Timestamp) -> bool {
        match *self {
            Pause::Off => false,
            Pause::Indefinite => true,
            Pause::Until(ms) => now.utc_ms < ms,
        }
    }
    pub fn for_one_hour(now: Timestamp) -> Self {
        Pause::Until(now.utc_ms + MS_PER_HOUR)
    }
    /// Until the next local midnight (in the offset in force now).
    pub fn until_tomorrow(now: Timestamp) -> Self {
        let next_midnight_local = local_midnight_ms(now.local_date()) + 24 * MS_PER_HOUR;
        Pause::Until(next_midnight_local - i64::from(now.offset_min) * MS_PER_MIN)
    }
    fn to_json(self) -> Value {
        match self {
            Pause::Off => json!("off"),
            Pause::Indefinite => json!("indefinite"),
            Pause::Until(ms) => json!({ "until_ms": ms }),
        }
    }
    fn from_json(v: &Value) -> Result<Self, SettingsError> {
        match v {
            Value::String(s) if s == "off" => Ok(Pause::Off),
            Value::String(s) if s == "indefinite" => Ok(Pause::Indefinite),
            Value::Object(o) => o
                .get("until_ms")
                .and_then(Value::as_i64)
                .map(Pause::Until)
                .ok_or_else(|| SettingsError::invalid(KEY_PAUSE, "expected {\"until_ms\": <int>}")),
            _ => Err(SettingsError::invalid(
                KEY_PAUSE,
                "expected \"off\", \"indefinite\" or {\"until_ms\": <int>}",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Explicit on/off per collector. Collectors missing here use `default_on`.
    pub collectors: BTreeMap<String, bool>,
    pub idle_threshold_secs: u32,
    pub active_without_input_apps: Vec<String>,
    pub active_without_input_timeout_secs: u32,
    pub exclude_apps: Vec<String>,
    pub pause: Pause,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            collectors: BTreeMap::new(),
            idle_threshold_secs: DEFAULT_IDLE_THRESHOLD_SECS,
            active_without_input_apps: Vec::new(),
            active_without_input_timeout_secs: DEFAULT_AWI_TIMEOUT_SECS,
            exclude_apps: Vec::new(),
            pause: Pause::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    UnknownKey(String),
    Invalid { key: String, reason: String },
}

impl SettingsError {
    fn invalid(key: &str, reason: &str) -> Self {
        Self::Invalid {
            key: key.to_string(),
            reason: reason.to_string(),
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(k) => write!(f, "unknown setting `{k}`"),
            Self::Invalid { key, reason } => write!(f, "invalid value for `{key}`: {reason}"),
        }
    }
}
impl std::error::Error for SettingsError {}

impl Settings {
    pub fn is_excluded(&self, bundle_id: &str) -> bool {
        self.exclude_apps.iter().any(|b| b == bundle_id)
    }

    pub fn is_active_without_input(&self, bundle_id: &str) -> bool {
        self.active_without_input_apps
            .iter()
            .any(|b| b == bundle_id)
    }

    pub fn collector_enabled(&self, name: &str, default_on: bool) -> bool {
        self.collectors.get(name).copied().unwrap_or(default_on)
    }

    /// Parse a `(key, raw value)` pair and apply it. `raw` is JSON; for
    /// convenience a bare word that isn't valid JSON is taken as a string.
    pub fn apply(&mut self, key: &str, raw: &str) -> Result<(), SettingsError> {
        let v: Value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        self.apply_value(key, &v)
    }

    fn apply_value(&mut self, key: &str, v: &Value) -> Result<(), SettingsError> {
        match key {
            KEY_IDLE_THRESHOLD => self.idle_threshold_secs = timeout(key, v)?,
            KEY_AWI_TIMEOUT => self.active_without_input_timeout_secs = timeout(key, v)?,
            KEY_AWI_APPS => self.active_without_input_apps = string_list(key, v)?,
            KEY_EXCLUDE => self.exclude_apps = string_list(key, v)?,
            KEY_PAUSE => self.pause = Pause::from_json(v)?,
            _ => {
                if let Some(name) = key
                    .strip_prefix("collector.")
                    .and_then(|r| r.strip_suffix(".enabled"))
                    .filter(|n| !n.is_empty())
                {
                    let on = v
                        .as_bool()
                        .ok_or_else(|| SettingsError::invalid(key, "expected true or false"))?;
                    self.collectors.insert(name.to_string(), on);
                } else {
                    return Err(SettingsError::UnknownKey(key.to_string()));
                }
            }
        }
        Ok(())
    }

    /// Build from stored rows; unknown or invalid rows are skipped (and returned)
    /// so a bad row never stops tracking.
    pub fn from_rows<'a>(
        rows: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> (Self, Vec<String>) {
        let mut s = Settings::default();
        let mut skipped = Vec::new();
        for (k, v) in rows {
            if k.starts_with(INTERNAL_PREFIX) {
                continue;
            }
            if let Err(e) = s.apply(k, v) {
                skipped.push(e.to_string());
            }
        }
        (s, skipped)
    }

    /// The stored JSON value for `key`, rendered as text.
    pub fn get_json(&self, key: &str) -> Result<String, SettingsError> {
        let v = match key {
            KEY_IDLE_THRESHOLD => json!(self.idle_threshold_secs),
            KEY_AWI_TIMEOUT => json!(self.active_without_input_timeout_secs),
            KEY_AWI_APPS => json!(self.active_without_input_apps),
            KEY_EXCLUDE => json!(self.exclude_apps),
            KEY_PAUSE => self.pause.to_json(),
            _ => {
                let name = key
                    .strip_prefix("collector.")
                    .and_then(|r| r.strip_suffix(".enabled"))
                    .ok_or_else(|| SettingsError::UnknownKey(key.to_string()))?;
                match self.collectors.get(name) {
                    Some(b) => json!(b),
                    None => Value::Null,
                }
            }
        };
        Ok(v.to_string())
    }

    /// All user-settable keys with their current JSON values (collectors only
    /// when explicitly set).
    pub fn to_rows(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = [
            KEY_IDLE_THRESHOLD,
            KEY_AWI_APPS,
            KEY_AWI_TIMEOUT,
            KEY_EXCLUDE,
            KEY_PAUSE,
        ]
        .iter()
        .map(|k| (k.to_string(), self.get_json(k).expect("known key")))
        .collect();
        for (name, on) in &self.collectors {
            rows.push((collector_enabled_key(name), json!(on).to_string()));
        }
        rows
    }

    pub fn pause_json(p: Pause) -> String {
        p.to_json().to_string()
    }
}

fn timeout(key: &str, v: &Value) -> Result<u32, SettingsError> {
    let n = v
        .as_u64()
        .ok_or_else(|| SettingsError::invalid(key, "expected a whole number of seconds"))?;
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&n) {
        return Err(SettingsError::invalid(key, "must be between 60 and 86400"));
    }
    Ok(n as u32)
}

fn string_list(key: &str, v: &Value) -> Result<Vec<String>, SettingsError> {
    let arr = v
        .as_array()
        .ok_or_else(|| SettingsError::invalid(key, "expected a JSON array of bundle ids"))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| SettingsError::invalid(key, "array items must be non-empty strings"))?;
        if !out.iter().any(|x| x == s) {
            out.push(s.trim().to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Clock;
    use crate::time::ManualClock;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.idle_threshold_secs, 180);
        assert_eq!(s.active_without_input_timeout_secs, 1800);
        assert!(s.collector_enabled("app_usage", true));
        assert!(!s.collector_enabled("git", false));
    }

    #[test]
    fn apply_and_roundtrip() {
        let mut s = Settings::default();
        s.apply("idle_threshold_secs", "120").unwrap();
        s.apply("exclude.apps", r#"["com.example.secret"]"#)
            .unwrap();
        s.apply("active_without_input.apps", r#"["com.apple.Preview"]"#)
            .unwrap();
        s.apply("collector.active_idle.enabled", "false").unwrap();
        s.apply("pause", "indefinite").unwrap();
        let rows = s.to_rows();
        let (back, skipped) =
            Settings::from_rows(rows.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        assert!(skipped.is_empty());
        assert_eq!(back, s);
        assert!(back.is_excluded("com.example.secret"));
        assert!(back.is_active_without_input("com.apple.Preview"));
        assert!(!back.collector_enabled("active_idle", true));
    }

    #[test]
    fn rejects_bad_values() {
        let mut s = Settings::default();
        assert!(s.apply("idle_threshold_secs", "5").is_err());
        assert!(s.apply("idle_threshold_secs", "\"abc\"").is_err());
        assert!(s.apply("exclude.apps", "com.x").is_err());
        assert!(s.apply("nope", "1").is_err());
        assert!(s.apply("pause", "sometimes").is_err());
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn bad_rows_are_skipped_not_fatal() {
        let (s, skipped) = Settings::from_rows([
            ("idle_threshold_secs", "abc"),
            ("internal.max_session_ms", "5"),
            ("exclude.apps", "[\"a\"]"),
        ]);
        assert_eq!(skipped.len(), 1);
        assert_eq!(s.exclude_apps, vec!["a".to_string()]);
    }

    #[test]
    fn pause_variants() {
        let c = ManualClock::at_local(2026, 11, 16, 22, 15, 0, 480);
        let now = c.now();
        assert!(!Pause::Off.is_active(now));
        assert!(Pause::Indefinite.is_active(now));
        let hour = Pause::for_one_hour(now);
        assert!(hour.is_active(now));
        assert!(!hour.is_active(now.plus_ms(MS_PER_HOUR)));
        // Until tomorrow = next local midnight = 1h45m away.
        assert_eq!(
            Pause::until_tomorrow(now),
            Pause::Until(now.utc_ms + 105 * MS_PER_MIN)
        );
        let mut s = Settings::default();
        s.apply("pause", r#"{"until_ms": 1234}"#).unwrap();
        assert_eq!(s.pause, Pause::Until(1234));
    }
}

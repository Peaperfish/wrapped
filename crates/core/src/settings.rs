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
//! | `git.folders`                        | array of folders (`~/...` or absolute)  | `["~/code"]` |
//! | `git.author_emails`                  | array of emails, or `null` = resolve from `git config --global user.email` when git is turned on | `null` |
//! | `photos.folders`                     | array of folders                        | `["~/Pictures"]` |
//! | `photos.extensions`                  | array of extensions (case-insensitive)  | jpg jpeg heic heif arw dng |
//! | `ai.sources`                         | array of `{id, tool, path, builtin, enabled, bookmark?}`: folders the AI usage collector reads | `[]` (tools found on this machine are added at the first scan) |
//! | `appearance.theme`                   | `"system"`, `"light"` or `"dark"`        | `"system"` |
//! | `appearance.accent`                  | `"green"`, `"blue"`, `"purple"`, `"orange"` or `"pink"` | `"green"` |
//! | `recaps.notify.week` / `.month` / `.year` | `true` / `false`: notify when that recap is ready | `false` |
//! | `onboarding.done`                    | `true` / `false`, or `null` = not decided yet (see `Engine::begin_onboarding`) | `null` |
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
pub const KEY_GIT_FOLDERS: &str = "git.folders";
pub const KEY_GIT_EMAILS: &str = "git.author_emails";
pub const KEY_PHOTOS_FOLDERS: &str = "photos.folders";
pub const KEY_PHOTOS_EXTENSIONS: &str = "photos.extensions";
pub const KEY_ONBOARDING_DONE: &str = "onboarding.done";
pub const KEY_AI_SOURCES: &str = "ai.sources";
pub const KEY_THEME: &str = "appearance.theme";
pub const KEY_ACCENT: &str = "appearance.accent";
pub const KEY_NOTIFY_WEEK: &str = "recaps.notify.week";
pub const KEY_NOTIFY_MONTH: &str = "recaps.notify.month";
pub const KEY_NOTIFY_YEAR: &str = "recaps.notify.year";
pub const THEMES: &[&str] = &["system", "light", "dark"];
pub const ACCENTS: &[&str] = &["green", "blue", "purple", "orange", "pink"];
pub const DEFAULT_PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "heic", "heif", "arw", "dng"];
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

/// A folder the AI usage collector reads, and the tool whose log format it
/// holds. Never removed automatically (see `crate::ai`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSource {
    /// Stable id (survives the path changing).
    pub id: String,
    /// A tool id from `crate::ai::TOOLS`.
    pub tool: String,
    /// Absolute folder (or `~/...`).
    pub path: String,
    /// Found by the collector (a tool's default folder) rather than added by the user.
    pub builtin: bool,
    pub enabled: bool,
    /// Hex of a folder bookmark (macOS custom folders), to find a moved folder.
    pub bookmark: Option<String>,
}

impl AiSource {
    fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id, "tool": self.tool, "path": self.path,
            "builtin": self.builtin, "enabled": self.enabled,
        });
        if let Some(b) = &self.bookmark {
            v["bookmark"] = json!(b);
        }
        v
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
    pub git_folders: Vec<String>,
    /// `None` until resolved (when git is turned on) or set by the user.
    pub git_author_emails: Option<Vec<String>>,
    pub photos_folders: Vec<String>,
    /// Lowercase, without the dot.
    pub photos_extensions: Vec<String>,
    /// `None` until the collector has decided (existing data) or the user has
    /// finished the first-launch checklist.
    pub onboarding_done: Option<bool>,
    pub ai_sources: Vec<AiSource>,
    /// One of `THEMES`.
    pub theme: String,
    /// One of `ACCENTS`.
    pub accent: String,
    /// Recap notifications per period kind (week, month, year).
    pub notify_week: bool,
    pub notify_month: bool,
    pub notify_year: bool,
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
            git_folders: vec!["~/code".to_string()],
            git_author_emails: None,
            photos_folders: vec!["~/Pictures".to_string()],
            photos_extensions: DEFAULT_PHOTO_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            onboarding_done: None,
            ai_sources: Vec::new(),
            theme: THEMES[0].to_string(),
            accent: ACCENTS[0].to_string(),
            notify_week: false,
            notify_month: false,
            notify_year: false,
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
            KEY_GIT_FOLDERS => self.git_folders = folder_list(key, v)?,
            KEY_PHOTOS_FOLDERS => self.photos_folders = folder_list(key, v)?,
            KEY_GIT_EMAILS => {
                self.git_author_emails = if v.is_null() {
                    None
                } else {
                    Some(email_list(key, v)?)
                }
            }
            KEY_PHOTOS_EXTENSIONS => self.photos_extensions = extension_list(key, v)?,
            KEY_AI_SOURCES => self.ai_sources = ai_sources(key, v)?,
            KEY_THEME => self.theme = one_of(key, v, THEMES)?,
            KEY_ACCENT => self.accent = one_of(key, v, ACCENTS)?,
            KEY_NOTIFY_WEEK | KEY_NOTIFY_MONTH | KEY_NOTIFY_YEAR => {
                let on = v
                    .as_bool()
                    .ok_or_else(|| SettingsError::invalid(key, "expected true or false"))?;
                match key {
                    KEY_NOTIFY_WEEK => self.notify_week = on,
                    KEY_NOTIFY_MONTH => self.notify_month = on,
                    _ => self.notify_year = on,
                }
            }
            KEY_ONBOARDING_DONE => {
                self.onboarding_done = if v.is_null() {
                    None
                } else {
                    Some(v.as_bool().ok_or_else(|| {
                        SettingsError::invalid(key, "expected true, false or null")
                    })?)
                }
            }
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
            KEY_GIT_FOLDERS => json!(self.git_folders),
            KEY_GIT_EMAILS => json!(self.git_author_emails),
            KEY_PHOTOS_FOLDERS => json!(self.photos_folders),
            KEY_PHOTOS_EXTENSIONS => json!(self.photos_extensions),
            KEY_THEME => json!(self.theme),
            KEY_ACCENT => json!(self.accent),
            KEY_NOTIFY_WEEK => json!(self.notify_week),
            KEY_NOTIFY_MONTH => json!(self.notify_month),
            KEY_NOTIFY_YEAR => json!(self.notify_year),
            KEY_ONBOARDING_DONE => json!(self.onboarding_done),
            KEY_AI_SOURCES => Value::Array(self.ai_sources.iter().map(AiSource::to_json).collect()),
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
            KEY_GIT_FOLDERS,
            KEY_GIT_EMAILS,
            KEY_PHOTOS_FOLDERS,
            KEY_PHOTOS_EXTENSIONS,
            KEY_ONBOARDING_DONE,
            KEY_AI_SOURCES,
            KEY_THEME,
            KEY_ACCENT,
            KEY_NOTIFY_WEEK,
            KEY_NOTIFY_MONTH,
            KEY_NOTIFY_YEAR,
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

fn one_of(key: &str, v: &Value, allowed: &[&str]) -> Result<String, SettingsError> {
    match v.as_str() {
        Some(s) if allowed.contains(&s) => Ok(s.to_string()),
        _ => Err(SettingsError::invalid(
            key,
            &format!("expected one of: {}", allowed.join(", ")),
        )),
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

fn strings<'a>(key: &str, v: &'a Value, what: &str) -> Result<Vec<&'a str>, SettingsError> {
    let arr = v
        .as_array()
        .ok_or_else(|| SettingsError::invalid(key, &format!("expected a JSON array of {what}")))?;
    arr.iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| SettingsError::invalid(key, "array items must be non-empty strings"))
        })
        .collect()
}

fn push_unique(out: &mut Vec<String>, s: String) {
    if !out.contains(&s) {
        out.push(s);
    }
}

/// Folders: `~`, `~/...` or absolute paths; trailing slashes removed.
fn folder_list(key: &str, v: &Value) -> Result<Vec<String>, SettingsError> {
    let mut out = Vec::new();
    for s in strings(key, v, "folders")? {
        let ok = has_home_prefix(s) || std::path::Path::new(s).is_absolute();
        if !ok {
            return Err(SettingsError::invalid(
                key,
                "folders must be absolute or start with ~/",
            ));
        }
        let t = s.trim_end_matches(['/', '\\']);
        push_unique(
            &mut out,
            if t.is_empty() {
                s.to_string()
            } else {
                t.to_string()
            },
        );
    }
    Ok(out)
}

/// `ai.sources`: objects with a plain id, a known tool and a folder.
fn ai_sources(key: &str, v: &Value) -> Result<Vec<AiSource>, SettingsError> {
    let arr = v
        .as_array()
        .ok_or_else(|| SettingsError::invalid(key, "expected a JSON array of sources"))?;
    let mut out: Vec<AiSource> = Vec::new();
    for item in arr {
        let text = |k: &str| item.get(k).and_then(Value::as_str).map(str::trim);
        let id = text("id")
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 32
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
            .ok_or_else(|| SettingsError::invalid(key, "every source needs a plain `id`"))?;
        let tool = text("tool")
            .filter(|t| crate::ai::tool(t).is_some())
            .ok_or_else(|| SettingsError::invalid(key, "unknown `tool`"))?;
        let path = text("path")
            .filter(|p| has_home_prefix(p) || std::path::Path::new(p).is_absolute())
            .ok_or_else(|| {
                SettingsError::invalid(key, "`path` must be absolute or start with ~/")
            })?;
        if out.iter().any(|s| s.id == id) {
            return Err(SettingsError::invalid(key, "duplicate source `id`"));
        }
        out.push(AiSource {
            id: id.to_string(),
            tool: tool.to_string(),
            path: path.to_string(),
            builtin: item
                .get("builtin")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            bookmark: text("bookmark")
                .filter(|b| !b.is_empty())
                .map(str::to_string),
        });
    }
    Ok(out)
}

/// Emails, compared case-insensitively, so stored lowercase.
fn email_list(key: &str, v: &Value) -> Result<Vec<String>, SettingsError> {
    let mut out = Vec::new();
    for s in strings(key, v, "emails")? {
        if !s.contains('@') || s.contains(char::is_whitespace) {
            return Err(SettingsError::invalid(key, "expected email addresses"));
        }
        push_unique(&mut out, s.to_lowercase());
    }
    Ok(out)
}

/// File extensions: optional leading dot, letters/digits, stored lowercase.
fn extension_list(key: &str, v: &Value) -> Result<Vec<String>, SettingsError> {
    let mut out = Vec::new();
    for s in strings(key, v, "extensions")? {
        let e = s.trim_start_matches('.').to_ascii_lowercase();
        if e.is_empty() || e.len() > 10 || !e.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(SettingsError::invalid(
                key,
                "extensions are 1-10 letters or digits, e.g. \"jpg\"",
            ));
        }
        push_unique(&mut out, e);
    }
    Ok(out)
}

/// Expand a leading `~` to the home directory.
/// `~`, `~/x` (and `~\x` on Windows).
fn has_home_prefix(s: &str) -> bool {
    s == "~" || home_rest(s).is_some()
}

fn home_rest(s: &str) -> Option<&str> {
    s.strip_prefix("~/")
        .or_else(|| cfg!(windows).then(|| s.strip_prefix("~\\")).flatten())
}

pub fn expand_home(folder: &str) -> Option<std::path::PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
    };
    if folder == "~" {
        home()
    } else if let Some(rest) = home_rest(folder) {
        Some(home()?.join(rest))
    } else {
        Some(std::path::PathBuf::from(folder))
    }
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
        s.apply("appearance.theme", "dark").unwrap();
        s.apply("appearance.accent", "\"purple\"").unwrap();
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
        assert!(s.apply("appearance.theme", "\"sepia\"").is_err());
        assert!(s.apply("appearance.accent", "3").is_err());
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
    fn scanned_collector_settings() {
        let mut s = Settings::default();
        assert_eq!(s.get_json("git.folders").unwrap(), r#"["~/code"]"#);
        assert_eq!(s.get_json("git.author_emails").unwrap(), "null");
        // An absolute path in this platform's form ("/tmp/x", "C:\\...\\x").
        let abs = std::env::temp_dir().join("x");
        let abs = abs.to_str().unwrap();
        s.apply(
            "git.folders",
            &json!(["~/code/", abs, "~/code"]).to_string(),
        )
        .unwrap();
        assert_eq!(s.git_folders, vec!["~/code", abs]);
        assert!(s.apply("git.folders", r#"["relative/dir"]"#).is_err());
        s.apply("git.author_emails", r#"["Me@Example.com"]"#)
            .unwrap();
        assert_eq!(s.git_author_emails, Some(vec!["me@example.com".into()]));
        assert!(s.apply("git.author_emails", r#"["nope"]"#).is_err());
        s.apply("photos.extensions", r#"[".JPG", "arw", "jpg"]"#)
            .unwrap();
        assert_eq!(s.photos_extensions, vec!["jpg", "arw"]);
        assert!(s.apply("photos.extensions", r#"["j/p"]"#).is_err());
        s.apply("photos.folders", r#"["~/Pictures", "~/Desktop"]"#)
            .unwrap();
        let rows = s.to_rows();
        let (back, skipped) =
            Settings::from_rows(rows.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        assert!(skipped.is_empty());
        assert_eq!(back, s);
        s.apply("git.author_emails", "null").unwrap();
        assert_eq!(s.git_author_emails, None);
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

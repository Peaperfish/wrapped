//! What the dashboard's commands return, as Rust types with generated
//! TypeScript twins in `ui/src/lib/types/` (ts-rs; regenerate with
//! `UPDATE_TS_TYPES=1 cargo test -p wrapped-dashboard ts_types`).
//!
//! `summary` returns `wrapped_core::stats::Summary::to_json()` unchanged (the
//! same JSON as `wrapped-collector stats range --json`); [`SummaryJson`] only
//! describes that shape, and a test checks it matches field for field.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use wrapped_core::collector::{Agg, Unit};

/// What a metric's values measure (`Unit::name`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum UnitName {
    Secs,
    Count,
    Bytes,
}

impl From<Unit> for UnitName {
    fn from(u: Unit) -> Self {
        match u {
            Unit::Seconds => Self::Secs,
            Unit::Count => Self::Count,
            Unit::Bytes => Self::Bytes,
        }
    }
}

/// How daily values combine over a range (`Agg::name`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum AggName {
    Sum,
    Max,
    Mean,
}

impl From<Agg> for AggName {
    fn from(a: Agg) -> Self {
        match a {
            Agg::Sum => Self::Sum,
            Agg::Max => Self::Max,
            Agg::Mean => Self::Mean,
        }
    }
}

// ---------- meta() ----------

/// A collector and whether it is turned on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct CollectorMeta {
    pub name: String,
    pub title: String,
    pub enabled: bool,
    /// What it records, in plain words (setup and Settings).
    pub description: String,
    /// Permission needed, or "None".
    pub permissions: String,
    pub cost: String,
    pub default_on: bool,
}

/// The settings the dashboard shows and edits (through the collector).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct SettingsMeta {
    pub exclude_apps: Vec<String>,
    pub git_folders: Vec<String>,
    /// `None` = not chosen yet (resolved from `git config` when git is on).
    pub git_author_emails: Option<Vec<String>>,
    pub photos_folders: Vec<String>,
    /// `"off"`, `"indefinite"` or `"until"` (a timed pause still running).
    pub pause: String,
    pub pause_until_ms: Option<i64>,
}

/// One author email found by `git_emails` (setup and Settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct GitEmail {
    pub email: String,
    pub commits: u32,
    pub repos: u32,
    /// The email from `git config --global user.email`.
    pub mine: bool,
}

/// What `export_data` wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ExportResult {
    pub path: String,
    pub sessions: u32,
    pub daily_metrics: u32,
    pub samples: u32,
    pub items: u32,
}

/// A metric the heatmap can colour by (a metric without keys of a collector
/// that is on). `id` is `"<collector>/<metric>"`, the argument of `series`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct HeatmapMetric {
    pub id: String,
    pub collector: String,
    pub metric: String,
    pub title: String,
    pub unit: UnitName,
    pub agg: AggName,
}

/// Everything the window needs before it asks for numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct Meta {
    /// Build id of this dashboard (short git hash, `-dirty` when uncommitted).
    pub build: String,
    pub db_path: String,
    /// False when there is no database file yet (the collector never ran).
    pub db_found: bool,
    /// Every built-in collector, in collector order.
    pub collectors: Vec<CollectorMeta>,
    /// Earliest local date with data from a collector that is on.
    pub earliest_date: Option<String>,
    /// Today's local date on this machine (`YYYY-MM-DD`).
    pub today: String,
    /// Latest moment the collectors that are on recorded anything (UTC ms;
    /// an open session's end is the last flush).
    pub last_flush_ms: Option<i64>,
    /// Heatmap metrics of the collectors that are on, screen time first.
    pub metrics: Vec<HeatmapMetric>,
    /// The first-launch checklist applies: onboarding is not done and nothing
    /// has been recorded yet (also true without a database file).
    pub setup: bool,
    pub settings: SettingsMeta,
}

// ---------- series(year, metric) ----------

/// An inclusive range of local dates (same shape as the summary's `range`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RangeJson {
    pub from: String,
    pub to: String,
    pub days: i64,
}

/// One value per day of a year, zeros for days without data (the
/// summary's `series` entries, plus the range they cover).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct YearSeries {
    pub collector: String,
    pub metric: String,
    pub title: String,
    pub unit: UnitName,
    pub agg: AggName,
    pub range: RangeJson,
    /// `days[i]` is the value of `range.from + i` days.
    pub days: Vec<f64>,
}

// ---------- summary(from, to): the shape of Summary::to_json() ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TotalJson {
    pub collector: String,
    pub metric: String,
    /// `""` for metrics without keys.
    pub key: String,
    pub value: f64,
    pub unit: UnitName,
    pub agg: AggName,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ComparisonJson {
    pub collector: String,
    pub metric: String,
    pub unit: UnitName,
    pub current: f64,
    pub previous: f64,
    pub change: f64,
    /// Percent change; `null` when only the previous value is zero.
    pub pct: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KeyValueJson {
    pub key: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TopJson {
    pub collector: String,
    pub metric: String,
    pub title: String,
    pub unit: UnitName,
    pub agg: AggName,
    /// Best first.
    pub keys: Vec<KeyValueJson>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HourProfileJson {
    pub collector: String,
    pub metric: String,
    pub unit: UnitName,
    /// 24 values, local hours 00..23.
    pub hours: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SeriesJson {
    pub collector: String,
    pub metric: String,
    pub title: String,
    pub unit: UnitName,
    pub agg: AggName,
    /// One value per day of the summary's range.
    pub days: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TimestampJson {
    pub utc_ms: i64,
    pub offset_min: i32,
    /// RFC 3339 in the moment's own offset.
    pub local: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LongestSessionJson {
    pub app: String,
    pub start: TimestampJson,
    pub end: TimestampJson,
    pub secs: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LatestNightJson {
    pub night: String,
    pub at: TimestampJson,
    /// Minutes after the evening's midnight (can exceed 24 h).
    pub clock_min: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StreakJson {
    pub days: i64,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RecordsJson {
    pub longest_session: Option<LongestSessionJson>,
    pub latest_night: Option<LatestNightJson>,
    pub active_streak: Option<StreakJson>,
    pub commit_streak: Option<StreakJson>,
}

/// The stats engine's result for one range (`stats range --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SummaryJson {
    pub range: RangeJson,
    pub previous: RangeJson,
    pub totals: Vec<TotalJson>,
    pub comparisons: Vec<ComparisonJson>,
    pub top: Vec<TopJson>,
    pub time_of_day: Vec<HourProfileJson>,
    pub series: Vec<SeriesJson>,
    pub records: RecordsJson,
}

/// Every exported type: (file name, TypeScript source). Integers are plain
/// `number`s (dates, counts and UTC ms all fit in 2^53).
pub fn typescript_files() -> Vec<(String, String)> {
    let cfg = ts_rs::Config::new().with_large_int("number");
    macro_rules! files {
        ($($t:ty),* $(,)?) => {
            vec![$((
                format!("{}.ts", <$t as TS>::name(&cfg)),
                <$t as TS>::export_to_string(&cfg).expect("exportable type"),
            )),*]
        };
    }
    files!(
        UnitName,
        AggName,
        CollectorMeta,
        SettingsMeta,
        GitEmail,
        ExportResult,
        HeatmapMetric,
        Meta,
        RangeJson,
        YearSeries,
        TotalJson,
        ComparisonJson,
        KeyValueJson,
        TopJson,
        HourProfileJson,
        SeriesJson,
        TimestampJson,
        LongestSessionJson,
        LatestNightJson,
        StreakJson,
        RecordsJson,
        SummaryJson,
    )
}

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
    /// `"system"`, `"light"` or `"dark"`.
    pub theme: String,
    /// `"green"`, `"blue"`, `"purple"`, `"orange"` or `"pink"`.
    pub accent: String,
    /// Recap notifications on (`recaps.notify.week|month|year`).
    pub notify_week: bool,
    pub notify_month: bool,
    pub notify_year: bool,
    /// What the collector last saw: `"granted"`, `"denied"`,
    /// `"not_determined"`, `"unavailable"` (not inside Wrapped.app), or
    /// `"unknown"` (never asked).
    pub notify_permission: String,
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

/// One AI tool and whether its default folder exists on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AiToolMeta {
    pub id: String,
    pub title: String,
    /// The tool writes token counts to disk (false: sessions only).
    pub tokens: bool,
    /// Checked against real logs (false: written from documentation).
    pub tested: bool,
    pub note: String,
    /// The tool's default folder exists.
    pub detected: bool,
    /// That folder (the first one found), for setup.
    pub detected_path: Option<String>,
    /// Log files found there.
    pub detected_files: u32,
}

/// One configured AI source (a folder) and what the last scan saw of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AiSourceMeta {
    pub id: String,
    pub tool: String,
    pub path: String,
    /// Found by the collector (the tool's default folder) rather than added by the user.
    pub builtin: bool,
    pub enabled: bool,
    /// `"ok"`, `"missing"`, `"error"` or `"new"` (not scanned yet).
    pub state: String,
    pub last_seen_ms: Option<i64>,
    pub last_scan_ms: Option<i64>,
    pub files: u32,
    pub requests: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct AiMeta {
    /// Every supported tool, in display order.
    pub tools: Vec<AiToolMeta>,
    pub sources: Vec<AiSourceMeta>,
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
    pub ai: AiMeta,
    pub drive_health: Option<DriveHealthMeta>,
}

/// The outcome of the latest attempt and the latest usable snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DriveHealthMeta {
    pub last_scan_ms: Option<i64>,
    pub status: Option<String>,
    pub snapshot: Option<DriveHealthSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct DriveHealthSnapshot {
    #[serde(default)]
    pub time_ms: i64,
    pub host_bytes_read: Option<u64>,
    pub host_bytes_written: Option<u64>,
    pub endurance_used_percent: Option<u8>,
    pub temperature_c: Option<i32>,
    pub power_on_hours: Option<u64>,
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
    /// Days of `range` / `previous` that have any data.
    pub data_days: u32,
    pub previous_data_days: u32,
    pub totals: Vec<TotalJson>,
    pub comparisons: Vec<ComparisonJson>,
    pub top: Vec<TopJson>,
    pub time_of_day: Vec<HourProfileJson>,
    pub series: Vec<SeriesJson>,
    pub records: RecordsJson,
}

// ---------- now_snapshot() ----------

/// One core's load (`now_snapshot` → `cpu_cores`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CoreLoadJson {
    /// 0-100; null until two samples exist.
    pub percent: Option<f64>,
    /// "P" (performance) or "E" (efficiency); null when unknown.
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempJson {
    pub label: String,
    pub celsius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FanJson {
    pub name: String,
    pub rpm: f64,
    pub min_rpm: Option<f64>,
    pub max_rpm: Option<f64>,
}

/// Live hardware numbers (`wrapped_core::now::NowSnapshot::to_json`, the same
/// JSON as `wrapped-collector now --json`). Every field can be null (show
/// "—"); rates are per second, sizes in bytes; rates are null on the first
/// call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NowSnapshotJson {
    /// UTC ms.
    pub ts_ms: Option<i64>,
    pub chip: Option<String>,
    pub cpu_percent: Option<f64>,
    pub cpu_cores: Option<Vec<CoreLoadJson>>,
    pub cpu_p_percent: Option<f64>,
    pub cpu_e_percent: Option<f64>,
    pub gpu_percent: Option<f64>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    /// "normal" | "warning" | "critical".
    pub mem_pressure: Option<String>,
    pub mem_compressed_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
    pub swap_total_bytes: Option<u64>,
    pub swap_in_per_sec: Option<f64>,
    pub swap_out_per_sec: Option<f64>,
    pub disk_read_per_sec: Option<f64>,
    pub disk_write_per_sec: Option<f64>,
    pub net_down_per_sec: Option<f64>,
    pub net_up_per_sec: Option<f64>,
    pub cpu_temp_c: Option<f64>,
    pub gpu_temp_c: Option<f64>,
    pub temps: Option<Vec<TempJson>>,
    pub fans: Option<Vec<FanJson>>,
}

// ---------- recaps ----------

/// One recap in the list (`wrapped_core::recap::RecapEntry::to_json`, the
/// same JSON as `wrapped-collector recap list --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RecapEntryJson {
    /// `week:2026-11-09`, `month:2026-11-01`, `year:2026-01-01`.
    pub id: String,
    /// "week" | "month" | "year".
    pub kind: String,
    pub label: String,
    /// The days covered (up to today while the period runs).
    pub range: RangeJson,
    pub in_progress: bool,
    pub active_days: u32,
    /// Null when app usage is off.
    pub screen_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppShareJson {
    /// App id (bundle id); `app_names` gives the display name.
    pub key: String,
    pub secs: f64,
    /// Of the period's screen time, 0..1.
    pub share: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WeekdayAvgJson {
    /// Monday = 0.
    pub weekday: u32,
    pub avg_secs: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RecapSessionJson {
    pub app: String,
    pub secs: f64,
    /// RFC 3339 in the session's own offset.
    pub start: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RecapNightJson {
    /// The evening's date.
    pub night: String,
    /// Minutes after that evening's midnight (26:30 = 1590).
    pub clock_min: i64,
}

/// A headline number of the summary slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HeadlineJson {
    /// "screen_time" | "top_app" | "commits" | "photos" | "ai_tokens" |
    /// "streak" | "disk_written" | "active_days".
    pub id: String,
    pub value: f64,
    /// The app id for `top_app`.
    pub text: Option<String>,
}

/// One story slide (`wrapped_core::recap::Slide::to_json`). A change is
/// null when the previous period is too thin or had nothing ("—").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
///
/// No `deny_unknown_fields` (ts-rs cannot read it on an enum); the recap
/// test's round trip catches a field this type would drop.
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlideJson {
    Intro {
        label: String,
        days: i64,
        active_days: u32,
        screen_secs: Option<f64>,
    },
    ScreenTime {
        total_secs: f64,
        daily_avg_secs: f64,
        screen_days: u32,
        change_pct: Option<f64>,
        busiest_weekday: Option<WeekdayAvgJson>,
    },
    TopApps {
        total_secs: f64,
        apps: Vec<AppShareJson>,
    },
    TimeOfDay {
        /// 24 values, local hours.
        hours: Vec<f64>,
        peak_hour: u32,
        /// "early_bird" | "daytime" | "night_owl".
        label: String,
        morning_share: f64,
        night_share: f64,
    },
    BusiestDay {
        date: String,
        /// Monday = 0.
        weekday: u32,
        secs: f64,
    },
    Records {
        longest_session: Option<RecapSessionJson>,
        latest_night: Option<RecapNightJson>,
        streak: Option<StreakJson>,
    },
    Git {
        commits: f64,
        change_pct: Option<f64>,
        top_repo: Option<KeyValueJson>,
        busiest_hour: Option<u32>,
        hours: Vec<f64>,
    },
    Photos {
        count: f64,
        change_pct: Option<f64>,
        top_camera: Option<KeyValueJson>,
        top_lens: Option<KeyValueJson>,
        /// Key: whole millimetres.
        top_focal: Option<KeyValueJson>,
    },
    Ai {
        tokens: f64,
        input: f64,
        output: f64,
        change_pct: Option<f64>,
        top_tool: Option<KeyValueJson>,
        top_model: Option<KeyValueJson>,
        top_project: Option<KeyValueJson>,
    },
    Disk {
        bytes_written: f64,
        change_pct: Option<f64>,
    },
    Summary {
        items: Vec<HeadlineJson>,
    },
}

/// A recap with its slides (`wrapped_core::recap::Recap::to_json`, the same
/// JSON as `wrapped-collector recap show … --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RecapJson {
    pub id: String,
    pub kind: String,
    pub label: String,
    /// The days covered.
    pub range: RangeJson,
    /// The whole period.
    pub period: RangeJson,
    pub in_progress: bool,
    /// What changes compare with.
    pub previous: RangeJson,
    /// False when the previous period is too thin (every change null).
    pub comparable: bool,
    pub active_days: u32,
    pub available: bool,
    pub slides: Vec<SlideJson>,
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
        AiToolMeta,
        AiSourceMeta,
        AiMeta,
        GitEmail,
        ExportResult,
        HeatmapMetric,
        Meta,
        DriveHealthMeta,
        DriveHealthSnapshot,
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
        CoreLoadJson,
        TempJson,
        FanJson,
        NowSnapshotJson,
        RecapEntryJson,
        AppShareJson,
        WeekdayAvgJson,
        RecapSessionJson,
        RecapNightJson,
        HeadlineJson,
        SlideJson,
        RecapJson,
    )
}

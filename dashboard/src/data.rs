//! The dashboard's data layer: plain functions over a database path, with no
//! Tauri in sight (the commands in `lib.rs` are thin wrappers, and the tests
//! call these directly).
//!
//! Every call opens the database read-only (`Store::open_read_only`: no
//! migration, no writes), runs its queries and closes it again. Nothing stays
//! open between calls, so no read transaction can hold back the collector's
//! WAL checkpoints, and settings changed in the meantime (a collector turned
//! off) apply to the next call.

use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use serde_json::Value;
use wrapped_core::stats::{DateRange, SUMMARY_TOP_N, Stats};
use wrapped_core::{Settings, Store, collectors};

use crate::types::{CollectorMeta, HeatmapMetric, Meta, RangeJson, YearSeries};

pub type Result<T> = std::result::Result<T, String>;

/// The database the collector uses: `WRAPPED_DB` if set, else the platform
/// data folder (macOS: `~/Library/Application Support/Wrapped/wrapped.db`).
/// `None` where the collector has no default yet (Windows until M9).
pub fn default_db_path() -> Option<PathBuf> {
    match std::env::var_os("WRAPPED_DB") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ if cfg!(target_os = "macos") => std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library/Application Support/Wrapped/wrapped.db")),
        _ => None,
    }
}

fn open(db: &Path) -> Result<(Store, Settings)> {
    if !db.exists() {
        return Err(format!("no database at {}", db.display()));
    }
    let store =
        Store::open_read_only(db).map_err(|e| format!("cannot open {}: {e}", db.display()))?;
    let (settings, _) = store.load_settings().map_err(|e| e.to_string())?;
    Ok((store, settings))
}

fn parse_date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("not a date (YYYY-MM-DD): {s:?}"))
}

fn range_json(r: DateRange) -> RangeJson {
    RangeJson {
        from: r.from.to_string(),
        to: r.to.to_string(),
        days: r.days(),
    }
}

/// Collectors, heatmap metrics, the earliest date with data and the last
/// flush. Without a database file: defaults, `db_found = false`.
pub fn meta(db: Option<&Path>, today: NaiveDate) -> Result<Meta> {
    let all = collectors::builtin();
    let opened = match db {
        Some(p) if p.exists() => Some(open(p)?),
        _ => None,
    };
    let settings = opened
        .as_ref()
        .map_or_else(Settings::default, |(_, s)| s.clone());
    let enabled: Vec<&'static str> = all
        .iter()
        .map(|c| c.info())
        .filter(|i| settings.collector_enabled(i.name, i.default_on))
        .map(|i| i.name)
        .collect();
    let collectors = all
        .iter()
        .map(|c| {
            let i = c.info();
            CollectorMeta {
                name: i.name.to_string(),
                title: i.title.to_string(),
                enabled: enabled.contains(&i.name),
            }
        })
        .collect();
    let metrics = all
        .iter()
        .filter(|c| enabled.contains(&c.info().name))
        .flat_map(|c| {
            let name = c.info().name;
            c.metrics()
                .into_iter()
                .filter(|m| m.key.is_none())
                .map(move |m| HeatmapMetric {
                    id: format!("{name}/{}", m.id),
                    collector: name.to_string(),
                    metric: m.id.to_string(),
                    title: m.title.to_string(),
                    unit: m.unit.into(),
                    agg: m.agg.into(),
                })
        })
        .collect();
    let (earliest_date, last_flush_ms) = match &opened {
        Some((store, _)) => (
            store
                .earliest_metric_date(&enabled)
                .map_err(|e| e.to_string())?
                .map(|d| d.to_string()),
            store
                .last_recorded_ms(&enabled)
                .map_err(|e| e.to_string())?,
        ),
        None => (None, None),
    };
    Ok(Meta {
        build: env!("WRAPPED_BUILD_ID").to_string(),
        db_path: db.map(|p| p.display().to_string()).unwrap_or_default(),
        db_found: opened.is_some(),
        collectors,
        earliest_date,
        today: today.to_string(),
        last_flush_ms,
        metrics,
    })
}

/// The stats engine's summary of `from..=to` (local dates), exactly as
/// `wrapped-collector stats range <from> <to> --json` prints it.
pub fn summary(db: &Path, from: &str, to: &str) -> Result<Value> {
    let range = DateRange::new(parse_date(from)?, parse_date(to)?).map_err(|e| e.to_string())?;
    let (store, settings) = open(db)?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    let sum = st
        .summary(range, SUMMARY_TOP_N)
        .map_err(|e| e.to_string())?;
    Ok(sum.to_json())
}

/// One value per day of `year` for a heatmap metric (`"<collector>/<metric>"`,
/// see `meta().metrics`), zeros for days without data.
pub fn series(db: &Path, year: i32, metric: &str) -> Result<YearSeries> {
    let (collector, id) = metric
        .split_once('/')
        .ok_or_else(|| format!("metric id must be <collector>/<metric>: {metric:?}"))?;
    let (Some(from), Some(to)) = (
        NaiveDate::from_ymd_opt(year, 1, 1),
        NaiveDate::from_ymd_opt(year, 12, 31),
    ) else {
        return Err(format!("year out of range: {year}"));
    };
    let range = DateRange::new(from, to).map_err(|e| e.to_string())?;
    let (store, settings) = open(db)?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    let def = st
        .metrics()
        .iter()
        .find(|m| m.collector == collector && m.def.id == id && m.def.key.is_none())
        .map(|m| m.def.clone())
        .ok_or_else(|| format!("{metric} is not a heatmap metric of a collector that is on"))?;
    let days = st
        .series(collector, id, None, range)
        .map_err(|e| e.to_string())?;
    debug_assert_eq!(days.first().map(|d| d.0.year()), Some(year));
    Ok(YearSeries {
        collector: collector.to_string(),
        metric: id.to_string(),
        title: def.title.to_string(),
        unit: def.unit.into(),
        agg: def.agg.into(),
        range: range_json(range),
        days: days.into_iter().map(|(_, v)| v).collect(),
    })
}

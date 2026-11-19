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
use wrapped_core::{Clock, Pause, Settings, Store, collectors};

use crate::types::{
    AiMeta, AiSourceMeta, AiToolMeta, CollectorMeta, ExportResult, GitEmail, HeatmapMetric, Meta,
    RangeJson, SettingsMeta, YearSeries,
};

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
                description: i.description.to_string(),
                permissions: i.permissions.to_string(),
                cost: i.cost.to_string(),
                default_on: i.default_on,
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
    let setup = match (&opened, settings.onboarding_done) {
        (_, Some(done)) => !done,
        (None, None) => true,
        (Some((store, _)), None) => !store.has_any_data().map_err(|e| e.to_string())?,
    };
    let now_ms = wrapped_core::SystemClock.now().utc_ms;
    let (pause, pause_until_ms) = match settings.pause {
        Pause::Indefinite => ("indefinite", None),
        Pause::Until(ms) if ms > now_ms => ("until", Some(ms)),
        _ => ("off", None),
    };
    let settings_meta = SettingsMeta {
        exclude_apps: settings.exclude_apps.clone(),
        git_folders: settings.git_folders.clone(),
        git_author_emails: settings.git_author_emails.clone(),
        photos_folders: settings.photos_folders.clone(),
        pause: pause.to_string(),
        pause_until_ms,
        theme: settings.theme.clone(),
        accent: settings.accent.clone(),
    };
    let ai = ai_meta(opened.as_ref().map(|(s, _)| s), &settings);
    Ok(Meta {
        build: env!("WRAPPED_BUILD_ID").to_string(),
        db_path: db.map(|p| p.display().to_string()).unwrap_or_default(),
        db_found: opened.is_some(),
        collectors,
        earliest_date,
        today: today.to_string(),
        last_flush_ms,
        metrics,
        setup,
        settings: settings_meta,
        ai,
    })
}

/// Supported AI tools (with what is detected on this machine) and the
/// configured sources with the status the last scan stored.
fn ai_meta(store: Option<&Store>, settings: &Settings) -> AiMeta {
    use wrapped_core::ai;
    let detected = ai::detect(&ai::Env::system());
    let tools = ai::TOOLS
        .iter()
        .map(|t| {
            let d = detected.iter().find(|d| d.tool.id == t.id);
            AiToolMeta {
                id: t.id.to_string(),
                title: t.title.to_string(),
                tokens: t.tokens,
                tested: t.tested,
                note: t.note.to_string(),
                detected: d.is_some(),
                detected_path: d.map(|d| d.root.display().to_string()),
                detected_files: d.map_or(0, |d| d.files as u32),
            }
        })
        .collect();
    let status = store
        .and_then(|s| {
            s.conn()
                .query_row(
                    "SELECT state FROM scan_state WHERE collector = 'ai_usage'",
                    [],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
        })
        .map(|j| ai::parse_status(&j))
        .unwrap_or_default();
    let sources = settings
        .ai_sources
        .iter()
        .map(|s| {
            let st = status.get(&s.id);
            AiSourceMeta {
                id: s.id.clone(),
                tool: s.tool.clone(),
                path: s.path.clone(),
                builtin: s.builtin,
                enabled: s.enabled,
                state: st
                    .map(|x| x.state.clone())
                    .filter(|x| !x.is_empty())
                    .unwrap_or_else(|| "new".to_string()),
                last_seen_ms: st.and_then(|x| x.last_seen_ms),
                last_scan_ms: st.and_then(|x| x.last_scan_ms),
                files: st.map_or(0, |x| x.files as u32),
                requests: st.map_or(0, |x| x.requests as u32),
            }
        })
        .collect();
    AiMeta { tools, sources }
}

/// Bundle ids the app usage collector has recorded, most time first (for the
/// exclude list's "add from recorded apps").
pub fn recorded_apps(db: &Path) -> Result<Vec<String>> {
    let (store, _) = open(db)?;
    let mut st = store
        .conn()
        .prepare(
            "SELECT key FROM daily_metrics WHERE collector = 'app_usage' AND metric = 'app_secs'
             GROUP BY key ORDER BY sum(value) DESC LIMIT 300",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Author emails in the repositories under `folders`, most commits first
/// (read-only: runs `git log` in each repository; stores nothing). The one
/// from `git config --global user.email` is marked.
pub fn git_emails(folders: &[String]) -> Vec<GitEmail> {
    let mine = wrapped_core::collectors::git::global_author_email();
    wrapped_core::collectors::git::author_email_report(folders)
        .into_iter()
        .map(|e| GitEmail {
            mine: mine.contains(&e.email),
            email: e.email,
            commits: e.commits as u32,
            repos: e.repos.len() as u32,
        })
        .collect()
}

/// Write everything to `path`: one JSON file (`kind = "json"`) or a folder
/// with one CSV per table (`"csv"`), like `wrapped-collector export`.
/// Read-only on the database.
pub fn export_data(db: &Path, kind: &str, path: &Path) -> Result<ExportResult> {
    let (store, _) = open(db)?;
    let c = match kind {
        "json" => {
            let f = std::fs::File::create(path)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            let mut w = std::io::BufWriter::new(f);
            store
                .export_json(&mut w, wrapped_core::SystemClock.now())
                .map_err(|e| e.to_string())?
        }
        "csv" => store.export_csv(path).map_err(|e| e.to_string())?,
        other => return Err(format!("export kind must be json or csv, not {other:?}")),
    };
    Ok(ExportResult {
        path: path.display().to_string(),
        sessions: c.sessions as u32,
        daily_metrics: c.daily_metrics as u32,
        samples: c.samples as u32,
        items: c.items as u32,
    })
}

/// The stats engine's summary of `from..=to` (local dates), exactly as
/// `wrapped-collector stats range <from> <to> --json` prints it. With
/// `vs_from` and `vs_to` (both or neither) the comparison runs against that
/// range instead of the previous range of equal length.
pub fn summary(
    db: &Path,
    from: &str,
    to: &str,
    vs_from: Option<&str>,
    vs_to: Option<&str>,
) -> Result<Value> {
    let range = DateRange::new(parse_date(from)?, parse_date(to)?).map_err(|e| e.to_string())?;
    let vs = match (vs_from, vs_to) {
        (Some(a), Some(b)) => {
            Some(DateRange::new(parse_date(a)?, parse_date(b)?).map_err(|e| e.to_string())?)
        }
        (None, None) => None,
        _ => return Err("vsFrom and vsTo go together".to_string()),
    };
    let (store, settings) = open(db)?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    let sum = st
        .summary_vs(range, vs, SUMMARY_TOP_N)
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

/// A sample from the shared Now sampler (created on first use). A poisoned
/// lock only means an earlier sample panicked: the sampler is still usable.
pub fn now_snapshot(sampler: &std::sync::Mutex<Option<wrapped_core::now::NowSampler>>) -> Value {
    let mut guard = sampler.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(wrapped_core::now::NowSampler::new)
        .sample()
        .to_json()
}

/// Every recap available on `today` (years, months, weeks; newest first), as
/// `wrapped-collector recap list --json` prints it.
pub fn recap_list(db: &Path, today: NaiveDate) -> Result<Value> {
    let (store, settings) = open(db)?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    let list = wrapped_core::recap::list(&st, today).map_err(|e| e.to_string())?;
    Ok(wrapped_core::recap::list_json(&list))
}

/// One recap by id (`week:2026-11-09`; any date inside the period works), as
/// `wrapped-collector recap show … --json` prints it.
pub fn recap_get(db: &Path, id: &str, today: NaiveDate) -> Result<Value> {
    let period = wrapped_core::recap::Period::parse_id(id)
        .ok_or_else(|| format!("not a recap id (week|month|year:YYYY-MM-DD): {id:?}"))?;
    let (store, settings) = open(db)?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    wrapped_core::recap::build(&st, period, today)
        .map_err(|e| e.to_string())?
        .map(|r| r.to_json())
        .ok_or_else(|| format!("{} has not started yet", period.label()))
}

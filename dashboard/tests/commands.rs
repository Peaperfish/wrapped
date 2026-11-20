//! The dashboard's three data commands (`meta`, `summary`, `series`) against
//! a fixture database, plus the read-only guarantees: nothing is written and
//! nothing stays open that would block the collector's WAL checkpoints.

mod fixture;

use fixture::*;
use rusqlite::Connection;
use serde_json::Value;
use wrapped_dashboard::data;
use wrapped_dashboard::types::{AggName, SummaryJson, UnitName};

fn total(sum: &Value, c: &str, m: &str, key: &str) -> f64 {
    sum["totals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["collector"] == c && t["metric"] == m && t["key"] == key)
        .map_or(0.0, |t| t["value"].as_f64().unwrap())
}

#[test]
fn meta_lists_collectors_metrics_and_data_bounds() {
    let dir = temp_dir("meta");
    let db = build(&dir);
    let m = data::meta(Some(&db), d("2026-03-11")).unwrap();
    assert!(m.db_found);
    assert_eq!(m.today, "2026-03-11");
    let on: Vec<(&str, bool)> = m
        .collectors
        .iter()
        .map(|c| (c.name.as_str(), c.enabled))
        .collect();
    assert_eq!(
        on,
        [
            ("app_usage", true),
            ("active_idle", true),
            ("git", true),
            ("photos", false),
            ("disk_memory", false),
            // On unless turned off: a default-on collector appears for an old database too.
            ("ai_usage", true),
            #[cfg(target_os = "macos")]
            ("drive_health", false),
        ]
    );
    // Only metrics without keys of collectors that are on; screen time first.
    let ids: Vec<&str> = m.metrics.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "app_usage/screen_time_secs",
            "app_usage/app_switches",
            "active_idle/away_secs",
            "active_idle/away_periods",
            "git/commits",
            "ai_usage/tokens",
            "ai_usage/requests",
            "ai_usage/sessions"
        ]
    );
    assert_eq!(m.metrics[0].unit, UnitName::Secs);
    assert_eq!(m.metrics[0].agg, AggName::Sum);
    assert_eq!(m.metrics[0].title, "Screen time");
    // The first commit (2025-12-29) is the earliest data of a collector that is on.
    assert_eq!(m.earliest_date.as_deref(), Some("2025-12-29"));
    // The open session's end: the last flush.
    assert_eq!(m.last_flush_ms, Some(last_flush().utc_ms));
    assert!(!m.build.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn meta_without_a_database_reports_defaults() {
    let dir = temp_dir("meta-none");
    let missing = dir.join("nothing.db");
    let m = data::meta(Some(&missing), d("2026-03-11")).unwrap();
    assert!(!m.db_found);
    assert!(!missing.exists(), "meta must not create the database");
    assert_eq!(m.earliest_date, None);
    assert_eq!(m.last_flush_ms, None);
    // Defaults: app usage and active / idle are on.
    assert_eq!(m.metrics[0].id, "app_usage/screen_time_secs");
    assert!(m.metrics.iter().all(|x| !x.id.starts_with("git/")));
    let m = data::meta(None, d("2026-03-11")).unwrap();
    assert!(!m.db_found);
    assert!(data::summary(&missing, "2026-01-01", "2026-01-31", None, None).is_err());
    assert!(data::series(&missing, 2026, "app_usage/screen_time_secs").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn series_is_one_value_per_day_of_the_year_with_zeros() {
    let dir = temp_dir("series");
    let db = build(&dir);
    let s = data::series(&db, 2026, "app_usage/screen_time_secs").unwrap();
    assert_eq!(
        (s.collector.as_str(), s.metric.as_str()),
        ("app_usage", "screen_time_secs")
    );
    assert_eq!(s.unit, UnitName::Secs);
    assert_eq!(
        (s.range.from.as_str(), s.range.to.as_str(), s.range.days),
        ("2026-01-01", "2026-12-31", 365)
    );
    assert_eq!(s.days.len(), 365);
    // Jan 1: the 75 minutes after midnight; Mar 8 (index 66): 2 h;
    // Mar 9: 50 min + 2 h 45 min; Mar 10: 2 h 30 min open session.
    assert_eq!(s.days[0], 75.0 * 60.0);
    assert_eq!(s.days[66], 2.0 * 3600.0);
    assert_eq!(s.days[67], (50.0 + 165.0) * 60.0);
    assert_eq!(s.days[68], 150.0 * 60.0);
    assert_eq!(s.days.iter().filter(|v| **v > 0.0).count(), 4);
    // The year's series adds up to the summary's total for the year.
    let sum = data::summary(&db, "2026-01-01", "2026-12-31", None, None).unwrap();
    assert_eq!(
        s.days.iter().sum::<f64>(),
        total(&sum, "app_usage", "screen_time_secs", "")
    );

    // A leap year without data: 366 zeros.
    let s = data::series(&db, 2024, "git/commits").unwrap();
    assert_eq!(s.days.len(), 366);
    assert!(s.days.iter().all(|v| *v == 0.0));
    let s = data::series(&db, 2025, "git/commits").unwrap();
    assert_eq!(s.days[362], 1.0); // 2025-12-29

    // Hidden (collector off), keyed or unknown metrics are refused.
    assert!(data::series(&db, 2026, "photos/shots").is_err());
    assert!(data::series(&db, 2026, "app_usage/app_secs").is_err());
    assert!(data::series(&db, 2026, "screen_time_secs").is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn summary_is_the_stats_engine_json_and_matches_the_ts_types() {
    let dir = temp_dir("summary");
    let db = build(&dir);
    let sum = data::summary(&db, "2025-12-30", "2026-03-10", None, None).unwrap();
    assert_eq!(sum["range"]["from"], "2025-12-30");
    assert_eq!(sum["range"]["days"], 71);
    // 1 h 30 + 30 min + 2 h 15 (over midnight) + 2 h + 50 min + 2 h 45 + 2 h 30.
    assert_eq!(
        total(&sum, "app_usage", "screen_time_secs", ""),
        (90.0 + 30.0 + 135.0 + 120.0 + 50.0 + 165.0 + 150.0) * 60.0
    );
    assert_eq!(total(&sum, "git", "commits", ""), 3.0);
    assert_eq!(total(&sum, "git", "commits_by_repo", "prism"), 1.0);
    // Photos are off: hidden although a shot is stored.
    assert!(
        sum["totals"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["collector"] != "photos")
    );
    assert_eq!(sum["records"]["commit_streak"]["days"], 2);
    assert_eq!(sum["records"]["active_streak"]["days"], 3);

    // The generated TypeScript types describe this JSON exactly: every
    // field present, nothing extra (deny_unknown_fields), same values.
    let typed: SummaryJson = serde_json::from_value(sum.clone()).unwrap();
    assert_same_json(&serde_json::to_value(&typed).unwrap(), &sum, "$");

    // Reversed or malformed ranges are errors, not panics.
    assert!(data::summary(&db, "2026-03-10", "2026-03-01", None, None).is_err());
    assert!(data::summary(&db, "March 1", "2026-03-10", None, None).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Equal JSON, numbers compared as f64 (`5` and `5.0` are the same value).
fn assert_same_json(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            assert_eq!(x.as_f64(), y.as_f64(), "{path}");
        }
        (Value::Array(x), Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "{path} length");
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                assert_same_json(p, q, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            let keys = |o: &serde_json::Map<String, Value>| {
                o.keys().cloned().collect::<std::collections::BTreeSet<_>>()
            };
            assert_eq!(keys(x), keys(y), "{path} keys");
            for (k, p) in x {
                assert_same_json(p, &y[k], &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}

fn file_bytes(p: &std::path::Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_default()
}

#[test]
fn commands_never_write_and_never_block_a_checkpoint() {
    let dir = temp_dir("readonly");
    let db = build(&dir);
    // The collector's connection, with a WAL holding uncheckpointed frames.
    let writer = Connection::open(&db).unwrap();
    writer
        .execute_batch(
            "PRAGMA wal_autocheckpoint = 0;
             INSERT INTO settings(key, value) VALUES ('internal.test', '1');",
        )
        .unwrap();
    let wal = db.with_extension("db-wal");
    let before = (file_bytes(&db), file_bytes(&wal));

    data::meta(Some(&db), d("2026-03-11")).unwrap();
    data::summary(&db, "2025-01-01", "2026-12-31", None, None).unwrap();
    data::series(&db, 2026, "app_usage/screen_time_secs").unwrap();

    assert!(
        before == (file_bytes(&db), file_bytes(&wal)),
        "a read changed the database or its WAL"
    );
    // A full checkpoint succeeds only if no reader still holds a snapshot.
    let (busy, _, _): (i64, i64, i64) = writer
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0, "a dashboard read kept a transaction open");
    drop(writer);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn summary_with_an_explicit_comparison_range() {
    let dir = temp_dir("summary-vs");
    let db = build(&dir);
    let plain = data::summary(&db, "2026-03-08", "2026-03-10", None, None).unwrap();
    assert_eq!(plain["previous"]["from"], "2026-03-05");
    let vs = data::summary(
        &db,
        "2026-03-08",
        "2026-03-10",
        Some("2026-03-01"),
        Some("2026-03-03"),
    )
    .unwrap();
    assert_eq!(vs["range"], plain["range"]);
    assert_eq!(vs["totals"], plain["totals"]);
    assert_eq!(vs["previous"]["from"], "2026-03-01");
    assert_eq!(vs["previous"]["to"], "2026-03-03");
    assert_eq!(vs["previous"]["days"], 3);
    // Only one of the two is an error, as is a reversed range.
    assert!(data::summary(&db, "2026-03-08", "2026-03-10", Some("2026-03-01"), None).is_err());
    assert!(
        data::summary(
            &db,
            "2026-03-08",
            "2026-03-10",
            Some("2026-03-03"),
            Some("2026-03-01")
        )
        .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

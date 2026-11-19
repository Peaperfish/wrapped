//! `recap_list` and `recap_get` return the JSON their TypeScript types
//! describe, field for field, for every slide kind.

mod fixture;

use fixture::*;
use rusqlite::Connection;
use serde_json::Value;
use wrapped_dashboard::data;
use wrapped_dashboard::types::{RecapEntryJson, RecapJson, SlideJson};

/// The fixture plus every collector on and a week (9-15 Mar 2026) with data
/// for each slide.
fn db_with_every_slide(dir: &std::path::Path) -> std::path::PathBuf {
    let db = build(dir);
    let c = Connection::open(&db).unwrap();
    for name in ["photos", "disk_memory", "ai_usage"] {
        c.execute(
            "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, 'true')",
            [format!("collector.{name}.enabled")],
        )
        .unwrap();
    }
    let put = |date: &str, col: &str, metric: &str, key: &str, v: f64| {
        c.execute(
            "INSERT OR REPLACE INTO daily_metrics(date, collector, metric, key, value) VALUES (?1,?2,?3,?4,?5)",
            (date, col, metric, key, v),
        )
        .unwrap();
    };
    for date in ["2026-03-11", "2026-03-12"] {
        put(date, "app_usage", "screen_time_secs", "", 7200.0);
        put(date, "app_usage", "app_secs", "com.apple.dt.Xcode", 7200.0);
        put(date, "app_usage", "active_secs_by_hour", "23", 7200.0);
        put(date, "git", "commits", "", 3.0);
        put(date, "git", "commits_by_repo", "wrapped", 3.0);
        put(date, "git", "commits_by_hour", "22", 3.0);
        put(date, "photos", "shots", "", 12.0);
        put(date, "photos", "shots_by_camera", "ILCE-7RM5", 12.0);
        put(date, "photos", "shots_by_lens", "FE 20mm", 12.0);
        put(date, "photos", "shots_by_focal_length", "20", 12.0);
        put(date, "ai_usage", "tokens", "", 5000.0);
        put(date, "ai_usage", "tokens_by_kind", "input", 1000.0);
        put(date, "ai_usage", "tokens_by_kind", "output", 4000.0);
        put(date, "ai_usage", "tokens_by_tool", "claude", 5000.0);
        put(
            date,
            "ai_usage",
            "tokens_by_model",
            "claude-opus-5-5",
            5000.0,
        );
        put(date, "ai_usage", "tokens_by_project", "wrapped", 5000.0);
        put(date, "disk_memory", "bytes_written", "", 4e9);
    }
    db
}

/// Every number as f64 (`5` and `5.0` are the same value).
fn floats(v: Value) -> Value {
    match v {
        Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(floats).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, floats(v))).collect()),
        other => other,
    }
}

#[test]
fn recap_list_and_get_match_the_ts_types() {
    let dir = temp_dir("recaps");
    let db = db_with_every_slide(&dir);
    let today = d("2026-03-20");
    let list = data::recap_list(&db, today).unwrap();
    let typed: Vec<RecapEntryJson> = serde_json::from_value(list.clone()).unwrap();
    assert_eq!(
        floats(serde_json::to_value(&typed).unwrap()),
        floats(list.clone())
    );
    let ids: Vec<&str> = typed.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"week:2026-03-09"), "{ids:?}");
    assert!(ids.contains(&"week:2025-12-29"), "{ids:?}");

    let mut kinds = std::collections::BTreeSet::new();
    for e in &typed {
        let v = data::recap_get(&db, &e.id, today).unwrap();
        let r: RecapJson = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(floats(serde_json::to_value(&r).unwrap()), floats(v.clone()));
        assert_eq!(r.id, e.id);
        assert!(r.available);
        for s in &r.slides {
            kinds.insert(
                serde_json::to_value(s).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        assert!(matches!(r.slides.first(), Some(SlideJson::Intro { .. })));
        assert!(matches!(r.slides.last(), Some(SlideJson::Summary { .. })));
    }
    let all: std::collections::BTreeSet<String> = wrapped_core::recap::SLIDE_KINDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(kinds, all);

    // Any date inside the period; bad ids and future periods are errors.
    let v = data::recap_get(&db, "week:2026-03-15", today).unwrap();
    assert_eq!(v["id"], "week:2026-03-09");
    assert!(data::recap_get(&db, "fortnight:2026-03-15", today).is_err());
    assert!(data::recap_get(&db, "week:2027-01-01", today).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn slide_export_only_writes_inside_the_wrapped_folder() {
    let dir = temp_dir("recap-export");
    let base = data::export_base(&dir);
    std::fs::create_dir_all(&base).unwrap();
    let png = b"\x89PNG\r\n\x1a\nrest";
    let p = data::save_recap_png(&base, "week-2026-11-09", "03-top_apps.png", png).unwrap();
    assert!(p.ends_with(std::path::Path::new("week-2026-11-09").join("03-top_apps.png")));
    assert_eq!(std::fs::read(&p).unwrap(), png);
    for (period, file) in [
        ("..", "01-intro.png"),
        ("week-2026-11-09/..", "01-intro.png"),
        ("week..2026", "01-intro.png"),
        ("week:2026-11-09", "01-intro.png"),
        ("Week-2026", "01-intro.png"),
        ("week-2026-11-09", "../01-intro.png"),
        ("week-2026-11-09", "01-intro.png.exe"),
        ("week-2026-11-09", "1-intro.png"),
        ("week-2026-11-09", "01-intro\\x.png"),
        ("", "01-intro.png"),
    ] {
        assert!(
            data::save_recap_png(&base, period, file, png).is_err(),
            "{period} {file}"
        );
    }
    // Not a PNG.
    assert!(data::save_recap_png(&base, "week-2026-11-09", "01-intro.png", b"<svg/>").is_err());
    // A folder link leading outside is refused.
    #[cfg(unix)]
    {
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("week-2026-01-05")).unwrap();
        assert!(data::save_recap_png(&base, "week-2026-01-05", "01-intro.png", png).is_err());
        assert!(!outside.join("01-intro.png").exists());
    }
    assert!(data::inside_export(&base, &p).is_ok());
    assert!(data::inside_export(&base, &dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

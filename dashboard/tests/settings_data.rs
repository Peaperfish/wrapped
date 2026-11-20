//! What the dashboard reads for setup and Settings: the setup flag, the
//! settings it shows, recorded apps and exports. All read-only.

mod fixture;

use fixture::*;
use rusqlite::Connection;
use wrapped_core::Store;
use wrapped_dashboard::data;

#[test]
fn setup_flag_follows_onboarding_and_data() {
    // No database at all: first launch.
    let dir = temp_dir("setup-none");
    let m = data::meta(Some(&dir.join("nope.db")), d("2026-03-11")).unwrap();
    assert!(m.setup && !m.db_found);

    // An empty database nobody has set up: setup.
    let dir = temp_dir("setup-empty");
    let db = dir.join("wrapped.db");
    drop(Store::open(&db).unwrap());
    assert!(data::meta(Some(&db), d("2026-03-11")).unwrap().setup);

    // Finished: not setup, whatever else.
    Store::open(&db)
        .unwrap()
        .set_setting("onboarding.done", "true")
        .unwrap();
    assert!(!data::meta(Some(&db), d("2026-03-11")).unwrap().setup);

    // An existing install (data, flag not stored yet): not setup.
    let dir = temp_dir("setup-existing");
    let db = build(&dir);
    assert!(!data::meta(Some(&db), d("2026-03-11")).unwrap().setup);
}

#[test]
fn meta_carries_collector_info_and_settings() {
    let dir = temp_dir("meta-settings");
    let db = build(&dir);
    let store = Store::open(&db).unwrap();
    store
        .set_setting("exclude.apps", r#"["com.example.a"]"#)
        .unwrap();
    store.set_setting("pause", "indefinite").unwrap();
    drop(store);
    let m = data::meta(Some(&db), d("2026-03-11")).unwrap();
    assert_eq!(m.settings.exclude_apps, ["com.example.a"]);
    assert_eq!(m.settings.pause, "indefinite");
    assert!(m.settings.pause_until_ms.is_none());
    let git = m.collectors.iter().find(|c| c.name == "git").unwrap();
    assert!(!git.default_on && !git.description.is_empty() && !git.permissions.is_empty());
}

#[test]
fn meta_lists_every_ai_tool_and_the_configured_sources() {
    let dir = temp_dir("meta-ai");
    let db = build(&dir);
    let missing = dir.join("nonexistent").join("x");
    let store = Store::open(&db).unwrap();
    store
        .set_setting(
            "ai.sources",
            &serde_json::json!([{"id":"abc","tool":"codex","path":missing.to_string_lossy(),"builtin":false,"enabled":false}]).to_string(),
        )
        .unwrap();
    drop(store);
    let m = data::meta(Some(&db), d("2026-03-11")).unwrap();
    assert_eq!(m.ai.tools.len(), 7);
    assert!(m.ai.tools.iter().all(|t| !t.title.is_empty()));
    let s = &m.ai.sources[0];
    assert_eq!((s.id.as_str(), s.tool.as_str()), ("abc", "codex"));
    assert!(!s.enabled && !s.builtin);
    assert_eq!(s.state, "new");
}

#[test]
fn export_writes_json_and_never_touches_the_database() {
    let dir = temp_dir("export");
    let db = build(&dir);
    let before = std::fs::read(&db).unwrap();
    let out = dir.join("out.json");
    let r = data::export_data(&db, "json", &out).unwrap();
    assert!(r.sessions > 0 && r.daily_metrics > 0);
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert!(v["sessions"].as_array().is_some_and(|a| !a.is_empty()));
    assert_eq!(std::fs::read(&db).unwrap(), before);
    assert!(data::export_data(&db, "xml", &out).is_err());
    let csv = dir.join("csv");
    std::fs::create_dir_all(&csv).unwrap();
    data::export_data(&db, "csv", &csv).unwrap();
    assert!(csv.join("sessions.csv").exists());
}

#[test]
fn recorded_apps_lists_bundle_ids_most_time_first() {
    let dir = temp_dir("apps");
    let db = build(&dir);
    let apps = data::recorded_apps(&db).unwrap();
    assert!(!apps.is_empty());
    let n: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT count(DISTINCT key) FROM daily_metrics WHERE collector='app_usage' AND metric='app_secs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(apps.len() as i64, n);
}

#[cfg(target_os = "macos")]
#[test]
fn drive_health_is_opt_in_and_keeps_the_latest_reading_after_a_failed_scan() {
    let dir = temp_dir("drive-health-meta");
    let db = dir.join("wrapped.db");
    let store = Store::open(&db).unwrap();
    let today = d("2026-03-11");
    let health = data::meta(Some(&db), today).unwrap();
    assert!(health.drive_health.is_none());
    assert!(
        health
            .collectors
            .iter()
            .any(|c| c.name == "drive_health" && !c.default_on)
    );

    store
        .set_setting("collector.drive_health.enabled", "true")
        .unwrap();
    let empty = data::meta(Some(&db), today).unwrap().drive_health.unwrap();
    assert!(empty.snapshot.is_none() && empty.status.is_none());

    store
        .conn()
        .execute_batch(
            r#"INSERT INTO items VALUES ('drive_health', '1000', 1000, 0, '{"temperature_c":31}');
           INSERT INTO items VALUES ('drive_health', '2000', 2000, 0, '{"temperature_c":33}');
           INSERT INTO scan_state VALUES ('drive_health', 3000, '"helper_missing"');"#,
        )
        .unwrap();
    let meta = data::meta(Some(&db), today).unwrap();
    assert!(!meta.metrics.iter().any(|m| m.collector == "drive_health"));
    let health = meta.drive_health.unwrap();
    assert_eq!(health.last_scan_ms, Some(3000));
    assert_eq!(health.status.as_deref(), Some("helper_missing"));
    let snapshot = health.snapshot.unwrap();
    assert_eq!(snapshot.time_ms, 2000);
    assert_eq!(snapshot.temperature_c, Some(33));
    assert_eq!(snapshot.host_bytes_read, None);
}

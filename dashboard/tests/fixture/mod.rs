//! A small database file written through the real store and the real
//! collectors' rollups, for the dashboard tests and the collector's
//! "dashboard JSON = CLI JSON" test (which includes this file by path).
//! Never the user's database: every fixture lives in its own temp folder.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use wrapped_core::collector::{DayInput, ItemRecord, MetricValue, OpenSession, SessionRecord};
use wrapped_core::settings::collector_enabled_key;
use wrapped_core::store::Batch;
use wrapped_core::time::{MS_PER_HOUR, MS_PER_MIN, local_midnight_ms};
use wrapped_core::{Store, Timestamp, collectors};

/// UTC+8, like the author's machine.
pub const OFF: i32 = 480;

pub fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// `h:m` local on `date`.
pub fn at(date: &str, h: i64, m: i64) -> Timestamp {
    let local = local_midnight_ms(d(date)) + h * MS_PER_HOUR + m * MS_PER_MIN;
    Timestamp::new(local - i64::from(OFF) * MS_PER_MIN, OFF)
}

/// A fresh, empty temp folder for one test.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wrapped-dashboard-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// When the fixture's open app session was last flushed.
pub fn last_flush() -> Timestamp {
    at("2026-03-10", 23, 30)
}

/// Writes `fixture.db` in `dir` and returns its path:
/// * app usage (on): sessions across the 2025/2026 new year (one over
///   midnight), a 3-day streak in March 2026 and an open session flushed at
///   [`last_flush`];
/// * active / idle (on): two idle periods;
/// * git (turned on): commits in two repositories;
/// * photos (off) with a shot, so hidden data exists.
pub fn build(dir: &Path) -> PathBuf {
    let path = dir.join("fixture.db");
    let mut store = Store::open(&path).unwrap();
    for (c, on) in [
        ("app_usage", true),
        ("active_idle", true),
        ("git", true),
        ("photos", false),
        ("disk_memory", false),
    ] {
        store
            .set_setting(&collector_enabled_key(c), if on { "true" } else { "false" })
            .unwrap();
    }
    let app = |key: &str, s: Timestamp, e: Timestamp| SessionRecord {
        collector: "app_usage",
        key: key.to_string(),
        start: s,
        end: e,
    };
    let closed = vec![
        app(
            "com.apple.Safari",
            at("2025-12-30", 9, 0),
            at("2025-12-30", 10, 30),
        ),
        app(
            "com.mitchellh.ghostty",
            at("2025-12-30", 10, 30),
            at("2025-12-30", 11, 0),
        ),
        // Over midnight into the new year.
        app(
            "com.mitchellh.ghostty",
            at("2025-12-31", 23, 0),
            at("2026-01-01", 1, 15),
        ),
        app(
            "com.apple.Safari",
            at("2026-03-08", 14, 0),
            at("2026-03-08", 16, 0),
        ),
        app(
            "com.mitchellh.ghostty",
            at("2026-03-09", 2, 10),
            at("2026-03-09", 3, 0),
        ),
        app(
            "com.mitchellh.ghostty",
            at("2026-03-09", 20, 0),
            at("2026-03-09", 22, 45),
        ),
        SessionRecord {
            collector: "active_idle",
            key: "idle".into(),
            start: at("2026-03-09", 12, 0),
            end: at("2026-03-09", 12, 40),
        },
        SessionRecord {
            collector: "active_idle",
            key: "sleep".into(),
            start: at("2026-03-10", 1, 0),
            end: at("2026-03-10", 8, 0),
        },
    ];
    let open = vec![(
        "app_usage",
        OpenSession {
            key: "com.apple.Safari".into(),
            start: at("2026-03-10", 21, 0),
        },
    )];
    let commit = |id: &str, t: Timestamp, repo: &str| ItemRecord {
        collector: "git",
        id: id.to_string(),
        time: t,
        offset_known: true,
        attrs: serde_json::json!({ "repo": repo }).to_string(),
    };
    let items = vec![
        commit("c1", at("2025-12-29", 16, 0), "wrapped"),
        commit("c2", at("2026-03-08", 16, 30), "wrapped"),
        commit("c3", at("2026-03-09", 17, 0), "wrapped"),
        commit("c4", at("2026-03-09", 18, 0), "prism"),
        ItemRecord {
            collector: "photos",
            id: "p1".into(),
            time: at("2026-03-08", 12, 0),
            offset_known: true,
            attrs: serde_json::json!({
                "make": "Sony", "model": "ILCE-7RM5", "lens": "FE 20mm", "focal_mm": 20.0,
            })
            .to_string(),
        },
    ];
    let all = collectors::builtin();
    let rollup = |name: &str, day: &DayInput<'_>| -> Option<Vec<MetricValue>> {
        all.iter()
            .find(|c| c.info().name == name)
            .map(|c| c.rollup(day))
    };
    let batch = Batch {
        closed: &closed,
        open: &open,
        samples: &[],
        items: &items,
        scans: &[],
    };
    store.flush(&batch, last_flush(), &rollup).unwrap();
    drop(store);
    path
}

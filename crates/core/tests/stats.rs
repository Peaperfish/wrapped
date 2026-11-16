//! Stats engine: totals, series, top-N, comparisons, time of day, records and
//! the range edge cases (DST, travel, photos without offset, empty ranges).

mod common;

use std::sync::Arc;

use common::*;
use wrapped_core::collector::{Agg, DayInput, DayItem, DaySample, Slice, Unit};
use wrapped_core::stats::{DateRange, Stats, pct_change, streak};
use wrapped_core::time::MS_PER_MIN;
use wrapped_core::{Engine, ManualClock, Store, SystemEvent, Timestamp, collectors};

const CST: i32 = 480;

fn range(a: &str, b: &str) -> DateRange {
    DateRange::new(d(a), d(b)).unwrap()
}

fn put(s: &Store, date: &str, collector: &str, metric: &str, key: &str, value: f64) {
    s.conn()
        .execute(
            "INSERT INTO daily_metrics(date, collector, metric, key, value) VALUES (?1,?2,?3,?4,?5)",
            (date, collector, metric, key, value),
        )
        .unwrap();
}

fn total(st: &Stats<'_>, r: DateRange, c: &str, m: &str, key: &str) -> Option<f64> {
    st.totals(r)
        .unwrap()
        .into_iter()
        .find(|t| t.collector == c && t.metric == m && t.key == key)
        .map(|t| t.value)
}

// ---------- metric definitions ----------

/// Rollup input that makes every collector emit every metric it has.
fn exercise_all_rollups() -> Vec<(&'static str, Vec<String>)> {
    let date = d("2026-11-16");
    let t = |h| at(date, h, 0, CST);
    let slices = vec![
        Slice {
            key: "com.a".into(),
            start: t(9),
            end: t(10),
            starts_session: true,
        },
        Slice {
            key: "idle".into(),
            start: t(10),
            end: t(11),
            starts_session: true,
        },
        Slice {
            key: "com.b".into(),
            start: t(11),
            end: t(12),
            starts_session: true,
        },
    ];
    let items: Vec<DayItem> = vec![DayItem {
        id: "x".into(),
        time: t(13),
        attrs: serde_json::json!({
            "repo": "r", "make": "Sony", "model": "A7", "lens": "L", "focal_mm": 35.0,
        }),
    }];
    let samples = vec![DaySample {
        metric: "reading".into(),
        time: t(14),
        value: 300.0,
        data: serde_json::json!({ "r": 1.0, "w": 2.0, "si": 3.0, "so": 4.0, "c": 5.0, "p": "warning" }),
    }];
    let input = DayInput {
        date,
        slices: &slices,
        items: &items,
        samples: &samples,
    };
    collectors::builtin()
        .iter()
        .map(|c| {
            let emitted = c.rollup(&input).into_iter().map(|m| m.metric.to_string());
            let mut v: Vec<String> = emitted.collect();
            v.sort();
            v.dedup();
            (c.info().name, v)
        })
        .collect()
}

#[test]
fn every_emitted_metric_is_declared_with_unit_and_aggregation() {
    let all = collectors::builtin();
    for (name, emitted) in exercise_all_rollups() {
        let c = all.iter().find(|c| c.info().name == name).unwrap();
        let defs = c.metrics();
        assert!(
            !emitted.is_empty(),
            "{name} emitted nothing; extend the test input"
        );
        for m in &emitted {
            let def = defs.iter().find(|d| d.id == m);
            assert!(
                def.is_some(),
                "{name}/{m} is emitted but not declared in metrics()"
            );
        }
        for def in &defs {
            assert!(
                emitted.iter().any(|m| m == def.id),
                "{name}/{} is declared but never emitted (stale definition?)",
                def.id
            );
            // By-hour and keyed breakdowns are additive across days.
            if def.id.ends_with("_by_hour") {
                assert!(def.key.is_some(), "{name}/{} has no key", def.id);
                assert_eq!(def.agg, Agg::Sum, "{name}/{}", def.id);
            }
            if def.id.ends_with("_secs") || def.id.contains("_secs_") {
                assert_eq!(def.unit, Unit::Seconds, "{name}/{}", def.id);
            }
            if def.id.contains("bytes") {
                assert_eq!(def.unit, Unit::Bytes, "{name}/{}", def.id);
            }
        }
        let mut ids: Vec<_> = defs.iter().map(|d| d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), defs.len(), "{name}: duplicate metric ids");
    }
}

#[test]
fn aggregations_of_existing_metrics() {
    let all = collectors::builtin();
    let agg = |c: &str, m: &str| {
        all.iter()
            .find(|x| x.info().name == c)
            .unwrap()
            .metrics()
            .into_iter()
            .find(|d| d.id == m)
            .unwrap()
            .agg
    };
    assert_eq!(agg("app_usage", "screen_time_secs"), Agg::Sum);
    assert_eq!(agg("git", "commits"), Agg::Sum);
    assert_eq!(agg("disk_memory", "compressed_peak_bytes"), Agg::Max);
    assert_eq!(agg("disk_memory", "compressed_avg_bytes"), Agg::Mean);
}

// ---------- totals, series, top, compare ----------

#[test]
fn totals_sum_max_and_mean_of_days_with_data() {
    let s = store_with(ALL);
    put(&s, "2026-11-01", "app_usage", "screen_time_secs", "", 100.0);
    put(&s, "2026-11-03", "app_usage", "screen_time_secs", "", 50.0);
    put(
        &s,
        "2026-11-01",
        "disk_memory",
        "compressed_peak_bytes",
        "",
        7.0,
    );
    put(
        &s,
        "2026-11-03",
        "disk_memory",
        "compressed_peak_bytes",
        "",
        9.0,
    );
    put(
        &s,
        "2026-11-01",
        "disk_memory",
        "compressed_avg_bytes",
        "",
        4.0,
    );
    put(
        &s,
        "2026-11-03",
        "disk_memory",
        "compressed_avg_bytes",
        "",
        8.0,
    );
    put(
        &s,
        "2026-11-05",
        "app_usage",
        "screen_time_secs",
        "",
        1000.0,
    ); // outside
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-01", "2026-11-04");
    assert_eq!(
        total(&st, r, "app_usage", "screen_time_secs", ""),
        Some(150.0)
    );
    assert_eq!(
        total(&st, r, "disk_memory", "compressed_peak_bytes", ""),
        Some(9.0)
    );
    // Mean of the two days with data, not of four days.
    assert_eq!(
        total(&st, r, "disk_memory", "compressed_avg_bytes", ""),
        Some(6.0)
    );
}

#[test]
fn series_has_every_day_with_zeros_and_filters_keys() {
    let s = store_with(ALL);
    put(&s, "2026-11-02", "app_usage", "screen_time_secs", "", 10.0);
    put(&s, "2026-11-02", "app_usage", "app_secs", "com.a", 6.0);
    put(&s, "2026-11-02", "app_usage", "app_secs", "com.b", 4.0);
    put(&s, "2026-11-03", "app_usage", "app_secs", "com.a", 1.0);
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-01", "2026-11-04");
    let v: Vec<f64> = st
        .series("app_usage", "screen_time_secs", None, r)
        .unwrap()
        .into_iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(v, vec![0.0, 10.0, 0.0, 0.0]);
    let a: Vec<f64> = st
        .series("app_usage", "app_secs", Some("com.a"), r)
        .unwrap()
        .into_iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(a, vec![0.0, 6.0, 1.0, 0.0]);
    let all: Vec<f64> = st
        .series("app_usage", "app_secs", None, r)
        .unwrap()
        .into_iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(all, vec![0.0, 10.0, 1.0, 0.0]);
    let dates: Vec<String> = st
        .series("app_usage", "screen_time_secs", None, r)
        .unwrap()
        .into_iter()
        .map(|x| x.0.to_string())
        .collect();
    assert_eq!(
        dates,
        ["2026-11-01", "2026-11-02", "2026-11-03", "2026-11-04"]
    );
    assert!(st.series("nope", "x", None, r).unwrap().is_empty());
}

#[test]
fn top_n_orders_by_value_then_key() {
    let s = store_with(ALL);
    for (date, key, v) in [
        ("2026-11-01", "repo-b", 3.0),
        ("2026-11-02", "repo-b", 2.0),
        ("2026-11-01", "repo-a", 5.0),
        ("2026-11-01", "repo-c", 1.0),
        ("2026-11-02", "repo-d", 5.0),
        ("2026-12-01", "repo-z", 99.0),
    ] {
        put(&s, date, "git", "commits_by_repo", key, v);
    }
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-01", "2026-11-30");
    let top = st.top("git", "commits_by_repo", r, 3).unwrap();
    assert_eq!(
        top,
        vec![
            ("repo-a".to_string(), 5.0),
            ("repo-b".to_string(), 5.0),
            ("repo-d".to_string(), 5.0),
        ]
    );
    assert_eq!(st.top("git", "commits_by_repo", r, 10).unwrap().len(), 4);
}

#[test]
fn compare_with_previous_period_of_equal_length() {
    let s = store_with(ALL);
    // Range Nov 8–14 (7 days); previous Nov 1–7.
    put(&s, "2026-11-01", "app_usage", "screen_time_secs", "", 100.0);
    put(&s, "2026-11-07", "app_usage", "screen_time_secs", "", 100.0);
    put(&s, "2026-11-08", "app_usage", "screen_time_secs", "", 300.0);
    put(
        &s,
        "2026-10-31",
        "app_usage",
        "screen_time_secs",
        "",
        5000.0,
    ); // too early
    put(&s, "2026-11-10", "git", "commits", "", 4.0); // previous had none
    put(&s, "2026-11-02", "photos", "shots", "", 9.0); // current has none
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-08", "2026-11-14");
    assert_eq!(r.previous(), range("2026-11-01", "2026-11-07"));
    let c = st.compare(r).unwrap();
    let get = |m: &str| c.iter().find(|x| x.metric == m).unwrap().clone();
    let screen = get("screen_time_secs");
    assert_eq!(
        (screen.current, screen.previous, screen.change),
        (300.0, 200.0, 100.0)
    );
    assert_eq!(screen.pct, Some(50.0));
    assert_eq!(get("commits").pct, None); // from zero: no percentage
    assert_eq!(get("commits").change, 4.0);
    assert_eq!(get("shots").pct, Some(-100.0));
    assert!(
        c.iter().all(|x| x.metric != "app_switches"),
        "no data either side"
    );
}

#[test]
fn pct_change_handles_zero() {
    assert_eq!(pct_change(0.0, 0.0), Some(0.0));
    assert_eq!(pct_change(5.0, 0.0), None);
    assert_eq!(pct_change(0.0, 5.0), Some(-100.0));
    assert_eq!(pct_change(15.0, 10.0), Some(50.0));
}

#[test]
fn time_of_day_profiles_from_by_hour_metrics() {
    let s = store_with(ALL);
    put(
        &s,
        "2026-11-01",
        "app_usage",
        "active_secs_by_hour",
        "09",
        600.0,
    );
    put(
        &s,
        "2026-11-02",
        "app_usage",
        "active_secs_by_hour",
        "09",
        400.0,
    );
    put(
        &s,
        "2026-11-02",
        "app_usage",
        "active_secs_by_hour",
        "23",
        60.0,
    );
    put(&s, "2026-11-02", "git", "commits_by_hour", "14", 3.0);
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let p = st.time_of_day(range("2026-11-01", "2026-11-02")).unwrap();
    let app = p
        .iter()
        .find(|x| x.metric == "active_secs_by_hour")
        .unwrap();
    assert_eq!(app.hours[9], 1000.0);
    assert_eq!(app.hours[23], 60.0);
    assert_eq!(app.hours.iter().sum::<f64>(), 1060.0);
    let git = p.iter().find(|x| x.metric == "commits_by_hour").unwrap();
    assert_eq!(git.hours[14], 3.0);
    let names: Vec<_> = p.iter().map(|x| x.metric).collect();
    assert!(names.contains(&"shots_by_hour") && names.contains(&"bytes_written_by_hour"));
}

#[test]
fn disabled_collectors_are_hidden() {
    let s = store_with(&["app_usage", "active_idle"]); // git off, data kept
    put(&s, "2026-11-01", "git", "commits", "", 4.0);
    put(&s, "2026-11-01", "git", "commits_by_repo", "r", 4.0);
    put(&s, "2026-11-01", "app_usage", "screen_time_secs", "", 10.0);
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-01", "2026-11-01");
    assert!(st.totals(r).unwrap().iter().all(|t| t.collector != "git"));
    assert!(st.top("git", "commits_by_repo", r, 5).unwrap().is_empty());
    assert!(st.series("git", "commits", None, r).unwrap().is_empty());
    assert!(st.compare(r).unwrap().iter().all(|c| c.collector != "git"));
    assert!(
        st.time_of_day(r)
            .unwrap()
            .iter()
            .all(|p| p.collector != "git")
    );
    assert_eq!(st.records(r).unwrap().commit_streak, None);
    let sum = st.summary(r, 5).unwrap();
    assert!(sum.top.iter().all(|(m, _)| m.collector != "git"));
    assert!(sum.series.iter().all(|(m, _)| m.collector != "git"));
    assert_eq!(
        total(&st, r, "app_usage", "screen_time_secs", ""),
        Some(10.0)
    );
}

// ---------- records ----------

#[test]
fn longest_session_joins_offset_split_and_uses_start_day() {
    let mut s = store_with(ALL);
    let day = d("2026-11-10");
    let mut p = Pending::default();
    p.session(
        "app_usage",
        "com.a",
        at(day, 9, 0, CST),
        at(day, 10, 0, CST),
    ); // 1 h
    // 50 min + 50 min, split by an offset change: one 100-min session.
    let mid = at(day, 11, 50, CST);
    p.session("app_usage", "com.b", at(day, 11, 0, CST), mid);
    p.session(
        "app_usage",
        "com.b",
        Timestamp::new(mid.utc_ms, 540),
        Timestamp::new(mid.utc_ms + 50 * MS_PER_MIN, 540),
    );
    // Different app straight after: not joined.
    p.session(
        "app_usage",
        "com.c",
        Timestamp::new(mid.utc_ms + 50 * MS_PER_MIN, 540),
        Timestamp::new(mid.utc_ms + 140 * MS_PER_MIN, 540),
    );
    // Longer, but starts the day before the range.
    p.session(
        "app_usage",
        "com.z",
        at(day, -3, 0, CST),
        at(day, 2, 0, CST),
    );
    p.flush(&mut s, at(day, 23, 0, CST));
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let rec = st.records(DateRange::day(day)).unwrap();
    let l = rec.longest_session.unwrap();
    assert_eq!(l.app, "com.b");
    assert_eq!(l.secs, 6000.0);
    assert_eq!(l.start, at(day, 11, 0, CST));
    // The day before sees the 5-hour session.
    let prev = st
        .records(DateRange::day(d("2026-11-09")))
        .unwrap()
        .longest_session
        .unwrap();
    assert_eq!((prev.app.as_str(), prev.secs), ("com.z", 5.0 * 3600.0));
}

#[test]
fn latest_night_counts_early_morning_as_the_night_before() {
    let mut s = store_with(ALL);
    let (d1, d2, d3) = (d("2026-11-10"), d("2026-11-11"), d("2026-11-12"));
    let mut p = Pending::default();
    p.session(
        "app_usage",
        "com.a",
        at(d1, 22, 0, CST),
        at(d1, 23, 40, CST),
    );
    // Night of the 11th: active until 02:15 on the 12th.
    p.session(
        "app_usage",
        "com.a",
        at(d2, 23, 30, CST),
        at(d3, 2, 15, CST),
    );
    // 06:00 on the 12th is morning, not a late night.
    p.session("app_usage", "com.a", at(d3, 6, 0, CST), at(d3, 6, 30, CST));
    p.flush(&mut s, at(d3, 23, 0, CST));
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let n = st
        .records(range("2026-11-10", "2026-11-12"))
        .unwrap()
        .latest_night
        .unwrap();
    assert_eq!(n.night, d2);
    assert_eq!(n.clock_min, 26 * 60 + 15);
    assert_eq!(n.at, at(d3, 2, 15, CST));
    // Only the 12th: the 02:15 belongs to the 11th's night, so the 12th's
    // latest is 06:30.
    let n12 = st
        .records(DateRange::day(d3))
        .unwrap()
        .latest_night
        .unwrap();
    assert_eq!((n12.night, n12.clock_min), (d3, 6 * 60 + 30));
    // Only the 10th.
    let n10 = st
        .records(DateRange::day(d1))
        .unwrap()
        .latest_night
        .unwrap();
    assert_eq!(n10.clock_min, 23 * 60 + 40);
}

#[test]
fn latest_night_session_crossing_five_am_is_cut_at_five() {
    let mut s = store_with(ALL);
    let (d1, d2) = (d("2026-11-10"), d("2026-11-11"));
    let mut p = Pending::default();
    p.session("app_usage", "com.a", at(d2, 3, 0, CST), at(d2, 7, 0, CST));
    p.flush(&mut s, at(d2, 23, 0, CST));
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let n = st
        .records(DateRange::day(d1))
        .unwrap()
        .latest_night
        .unwrap();
    assert_eq!((n.night, n.clock_min), (d1, 29 * 60)); // until 05:00
    let n2 = st
        .records(DateRange::day(d2))
        .unwrap()
        .latest_night
        .unwrap();
    assert_eq!((n2.night, n2.clock_min), (d2, 7 * 60));
}

#[test]
fn streaks_of_active_days_and_commits() {
    let s = store_with(ALL);
    for day in [
        "2026-11-01",
        "2026-11-02",
        "2026-11-04",
        "2026-11-05",
        "2026-11-06",
    ] {
        put(&s, day, "app_usage", "screen_time_secs", "", 60.0);
    }
    put(&s, "2026-11-03", "app_usage", "screen_time_secs", "", 0.0);
    for day in ["2026-11-02", "2026-11-03"] {
        put(&s, day, "git", "commits", "", 1.0);
    }
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let rec = st.records(range("2026-11-01", "2026-11-30")).unwrap();
    let a = rec.active_streak.unwrap();
    assert_eq!(
        (a.days, a.from, a.to),
        (3, d("2026-11-04"), d("2026-11-06"))
    );
    let c = rec.commit_streak.unwrap();
    assert_eq!(
        (c.days, c.from, c.to),
        (2, d("2026-11-02"), d("2026-11-03"))
    );
    // A range cuts a streak.
    let cut = st.records(range("2026-11-05", "2026-11-30")).unwrap();
    assert_eq!(cut.active_streak.unwrap().days, 2);
}

#[test]
fn streak_ties_keep_the_earliest() {
    let s = |v: &[f64]| -> Vec<(chrono::NaiveDate, f64)> {
        v.iter()
            .enumerate()
            .map(|(i, x)| (d("2026-01-01") + chrono::Duration::days(i as i64), *x))
            .collect()
    };
    assert_eq!(streak(&s(&[0.0, 0.0])), None);
    let st = streak(&s(&[1.0, 1.0, 0.0, 1.0, 1.0])).unwrap();
    assert_eq!((st.days, st.from), (2, d("2026-01-01")));
}

// ---------- range edge cases ----------

#[test]
fn reversed_range_is_an_error_and_empty_data_gives_zeros() {
    assert!(DateRange::new(d("2026-11-02"), d("2026-11-01")).is_err());
    let s = store_with(ALL);
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2026-11-01", "2026-11-03");
    assert!(st.totals(r).unwrap().is_empty());
    assert!(st.compare(r).unwrap().is_empty());
    assert_eq!(
        st.series("app_usage", "screen_time_secs", None, r)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(st.records(r).unwrap(), Default::default());
    assert!(
        st.time_of_day(r)
            .unwrap()
            .iter()
            .all(|p| p.hours == [0.0; 24])
    );
}

#[test]
fn range_before_any_data() {
    let mut s = store_with(ALL);
    synth(&mut s, d("2026-06-01"), 10, CST, small());
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let r = range("2025-01-01", "2025-12-31");
    assert!(st.totals(r).unwrap().is_empty());
    assert_eq!(st.records(r).unwrap(), Default::default());
    let sum = st.summary(r, 5).unwrap();
    assert!(sum.series.iter().all(|(_, v)| v.iter().all(|x| x.1 == 0.0)));
    assert!(sum.top.iter().all(|(_, v)| v.is_empty()));
}

fn small() -> Scale {
    Scale {
        app_sessions: 20,
        commits_max: 4,
        photos_max: 5,
        readings: 12,
    }
}

#[test]
fn single_day_and_full_year_agree_with_the_daily_rows() {
    let mut s = store_with(ALL);
    synth(&mut s, d("2028-01-01"), 366, CST, small()); // leap year
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let year = range("2028-01-01", "2028-12-31");
    assert_eq!(year.days(), 366);
    let series = st
        .series("app_usage", "screen_time_secs", None, year)
        .unwrap();
    assert_eq!(series.len(), 366);
    let from_series: f64 = series.iter().map(|x| x.1).sum();
    let yt = total(&st, year, "app_usage", "screen_time_secs", "").unwrap();
    assert!((from_series - yt).abs() < 1e-6);
    // Sum of single-day totals = year total, for every additive metric.
    let mut by_days = 0.0;
    for day in year.dates() {
        by_days += total(&st, DateRange::day(day), "git", "commits", "").unwrap_or(0.0);
    }
    assert_eq!(by_days, total(&st, year, "git", "commits", "").unwrap());
    let items: f64 = s
        .conn()
        .query_row(
            "SELECT count(*) FROM items WHERE collector='git'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap() as f64;
    assert_eq!(by_days, items);
    // Per-app top sums to screen time.
    let apps: f64 = st
        .top("app_usage", "app_secs", year, 1000)
        .unwrap()
        .iter()
        .map(|x| x.1)
        .sum();
    assert!((apps - yt).abs() < 1e-3);
    let one = st.summary(DateRange::day(d("2028-02-29")), 3).unwrap();
    assert!(one.series.iter().all(|(_, v)| v.len() == 1));
}

/// Engine end-to-end: one session per local day around a DST change and a
/// flight; screen time per day and over the range must equal the wall-clock
/// time used, each second counted once.
#[test]
fn dst_and_offset_change_count_each_second_once() {
    let c = ManualClock::at_local(2026, 10, 31, 22, 0, 0, -240); // EDT
    let mut e = Engine::new(
        Store::open_in_memory().unwrap(),
        Arc::new(c.clone()),
        collectors::builtin(),
    )
    .unwrap();
    let tick = |e: &mut Engine| {
        e.handle(SystemEvent::IdleTick {
            secs_since_input: 1.0,
            display_assertion_holders: vec![],
        })
        .unwrap();
    };
    e.handle(SystemEvent::FrontApp {
        bundle_id: "com.a".into(),
        name: "A".into(),
    })
    .unwrap();
    // 22:00 Oct 31 EDT → active for 5 h of wall time; at 2:00 on Nov 1 the
    // clock falls back to EST (-300) after 4 h of wall time.
    for m in 1..=300 {
        c.advance_secs(60);
        if m == 240 {
            c.set_offset_min(-300);
        }
        tick(&mut e);
    }
    // A flight: jump to UTC+1 and keep going 2 h.
    c.set_offset_min(60);
    for _ in 0..120 {
        c.advance_secs(60);
        tick(&mut e);
    }
    e.handle(SystemEvent::WillSleep).unwrap();
    let st = Stats::new(e.store(), &collectors::builtin(), e.settings());
    let r = range("2026-10-30", "2026-11-03");
    let total_secs = total(&st, r, "app_usage", "screen_time_secs", "").unwrap();
    assert_eq!(
        total_secs,
        7.0 * 3600.0,
        "7 h of wall time, each second once"
    );
    let series = st.series("app_usage", "screen_time_secs", None, r).unwrap();
    let sum: f64 = series.iter().map(|x| x.1).sum();
    assert_eq!(sum, total_secs);
    // Oct 31 got 22:00–24:00 EDT = 2 h; the rest is on Nov 1 (local dates).
    assert_eq!(series[1], (d("2026-10-31"), 7200.0));
    assert_eq!(series[2].1, 5.0 * 3600.0);
    // Hours of day add up too.
    let p = st.time_of_day(r).unwrap();
    let hours: f64 = p
        .iter()
        .find(|x| x.metric == "active_secs_by_hour")
        .unwrap()
        .hours
        .iter()
        .sum();
    assert_eq!(hours, total_secs);
    let longest = st.records(r).unwrap().longest_session.unwrap();
    assert_eq!(longest.secs, 7.0 * 3600.0, "offset splits are one session");
}

#[test]
fn photos_without_offset_count_on_their_camera_date() {
    let mut s = store_with(ALL);
    let day = d("2026-11-10");
    let mut p = Pending::default();
    // Camera time 23:30 with no offset: stored naive, counts on the 10th
    // whatever the computer's zone.
    p.photo(
        "a",
        Timestamp::new(at(day, 23, 30, 0).utc_ms, 0),
        false,
        "A7",
        "L",
    );
    p.photo("b", at(day, 23, 30, CST), true, "A7", "L");
    p.flush(&mut s, at(day, 23, 59, CST));
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    assert_eq!(
        total(&st, DateRange::day(day), "photos", "shots", ""),
        Some(2.0)
    );
    let tod = st.time_of_day(DateRange::day(day)).unwrap();
    let h = tod.iter().find(|x| x.metric == "shots_by_hour").unwrap();
    assert_eq!(h.hours[23], 2.0);
    assert_eq!(
        total(&st, DateRange::day(d("2026-11-11")), "photos", "shots", ""),
        None
    );
}

#[test]
fn summary_json_has_every_part() {
    let mut s = store_with(ALL);
    synth(&mut s, d("2026-11-01"), 14, CST, small());
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let j = st
        .summary(range("2026-11-08", "2026-11-14"), 3)
        .unwrap()
        .to_json();
    for k in [
        "range",
        "previous",
        "totals",
        "comparisons",
        "top",
        "time_of_day",
        "series",
        "records",
    ] {
        assert!(!j[k].is_null(), "{k} missing");
    }
    assert_eq!(j["range"]["days"], 7);
    assert_eq!(j["previous"]["from"], "2026-11-01");
    let screen = j["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["metric"] == "screen_time_secs")
        .unwrap();
    assert_eq!(screen["days"].as_array().unwrap().len(), 7);
    assert_eq!(screen["unit"], "secs");
    assert!(j["records"]["longest_session"]["secs"].as_f64().unwrap() > 0.0);
}

#[test]
fn read_only_open_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("wrapped-stats-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.db");
    {
        let mut s = Store::open(&path).unwrap();
        synth(&mut s, d("2026-11-01"), 3, CST, small());
        s.conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let ro = Store::open_read_only(&path).unwrap();
    let st = Stats::new(&ro, &collectors::builtin(), &ro.load_settings().unwrap().0);
    let sum = st.summary(range("2026-11-01", "2026-11-03"), 5).unwrap();
    assert!(!sum.totals.is_empty());
    assert!(ro.set_setting("idle_threshold_secs", "240").is_err());
    drop(ro);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn latest_night_sees_a_long_session_that_started_well_before_the_range() {
    let mut s = store_with(ALL);
    let (d0, d2) = (d("2026-11-08"), d("2026-11-10"));
    let mut p = Pending::default();
    // 36 h without a break (e.g. an app on the active-without-input list),
    // from 20:00 on the 8th to 08:00 on the 10th.
    p.session("app_usage", "com.a", at(d0, 20, 0, CST), at(d2, 8, 0, CST));
    p.flush(&mut s, at(d2, 9, 0, CST));
    let st = Stats::new(&s, &collectors::builtin(), &settings(&s));
    let rec = st.records(DateRange::day(d2)).unwrap();
    let n = rec.latest_night.unwrap();
    assert_eq!((n.night, n.clock_min), (d2, 8 * 60));
    // It started on the 8th, so it is not the 10th's longest session.
    assert_eq!(rec.longest_session, None);
}

//! Stats engine timing over a synthetic year. Ignored by default (a
//! measurement, not a check); run with:
//!
//!     cargo test -p wrapped-core --release --test stats_perf -- --ignored --nocapture
//!
//! Data: 365 days for every collector through the real store and rollups:
//! 250 app / idle sessions a day (6 days in 7), a sleep period each night,
//! 0–12 commits a day over 6 repos, photo days (1 in 4) with 0–60 shots, and
//! 192 Disk & Memory readings a day. Written to a file database, then
//! reopened (fresh connection, OS cache warm). Each operation runs 7 times;
//! the median and the slowest run are printed. Target: < 50 ms each.

mod common;

use std::time::Instant;

use common::*;
use wrapped_core::Store;
use wrapped_core::recap;
use wrapped_core::stats::{DateRange, Stats};

/// One timed stats call; returns a result count so the work is not optimized away.
type Op<'a> = Box<dyn Fn(&Stats<'_>) -> usize + 'a>;

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, f64, T) {
    let mut ms = Vec::new();
    let mut last = None;
    for _ in 0..runs {
        let t = Instant::now();
        last = Some(f());
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    (ms[ms.len() / 2], ms[ms.len() - 1], last.unwrap())
}

#[test]
#[ignore]
fn stats_over_a_synthetic_year() {
    let dir = std::env::temp_dir().join(format!("wrapped-stats-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wrapped.db");
    let t0 = Instant::now();
    {
        let mut s = Store::open(&path).unwrap();
        for c in ALL {
            s.set_setting(&format!("collector.{c}.enabled"), "true")
                .unwrap();
        }
        synth(&mut s, d("2025-11-17"), 365, 480, REALISTIC);
        s.conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let gen_s = t0.elapsed().as_secs_f64();
    let s = Store::open_read_only(&path).unwrap();
    let count = |t: &str| -> i64 {
        s.conn()
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap()
    };
    let size = std::fs::metadata(&path).unwrap().len();
    println!(
        "data: {} sessions, {} items, {} samples, {} daily_metrics rows; db {:.1} MB; generated in {gen_s:.1} s",
        count("sessions"),
        count("items"),
        count("samples"),
        count("daily_metrics"),
        size as f64 / 1_048_576.0
    );
    let plan: Vec<String> = {
        let mut st = s
            .conn()
            .prepare(
                "EXPLAIN QUERY PLAN SELECT date, collector, metric, key, value FROM daily_metrics
                 WHERE date >= '2025-11-17' AND date <= '2026-11-16'
                   AND ('app_usage' IS NULL OR (collector = 'app_usage' AND metric = 'app_secs'))",
            )
            .unwrap();
        st.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    println!(
        "query plan (daily_metrics, one metric): {}",
        plan.join(" | ")
    );

    let settings = s.load_settings().unwrap().0;
    let all = collectors_all();
    let year = DateRange::new(d("2025-11-17"), d("2026-11-16")).unwrap();
    let month = DateRange::new(d("2026-10-17"), d("2026-11-16")).unwrap();
    const RUNS: usize = 7;
    let (m, w, st) = time(RUNS, || Stats::new(&s, &all, &settings));
    println!("{:<44} median {m:>7.2} ms   max {w:>7.2} ms", "Stats::new");
    let ops: Vec<(&str, Op)> = vec![
        (
            "year totals (all metrics, all keys)",
            Box::new(|st| st.totals(year).unwrap().len()),
        ),
        (
            "year heatmap series (screen time)",
            Box::new(|st| {
                st.series("app_usage", "screen_time_secs", None, year)
                    .unwrap()
                    .len()
            }),
        ),
        (
            "year heatmap series (GB written)",
            Box::new(|st| {
                st.series("disk_memory", "bytes_written", None, year)
                    .unwrap()
                    .len()
            }),
        ),
        (
            "year top-10 apps",
            Box::new(|st| st.top("app_usage", "app_secs", year, 10).unwrap().len()),
        ),
        (
            "year top-5 repos",
            Box::new(|st| st.top("git", "commits_by_repo", year, 5).unwrap().len()),
        ),
        (
            "year compare vs previous year",
            Box::new(|st| st.compare(year).unwrap().len()),
        ),
        (
            "year time of day",
            Box::new(|st| st.time_of_day(year).unwrap().len()),
        ),
        (
            "year records (sessions, night, streaks)",
            Box::new(|st| usize::from(st.records(year).unwrap().longest_session.is_some())),
        ),
        (
            "year summary (everything)",
            Box::new(|st| st.summary(year, 10).unwrap().totals.len()),
        ),
        (
            "year recap (2026 so far, every slide)",
            Box::new(|st| {
                let y = recap::Period::containing(recap::PeriodKind::Year, d("2026-06-01"));
                recap::build(st, y, d("2026-12-01"))
                    .unwrap()
                    .map_or(0, |r| r.slides.len())
            }),
        ),
        (
            "recap list (whole history)",
            Box::new(|st| recap::list(st, d("2026-12-01")).unwrap().len()),
        ),
        (
            "month summary (everything)",
            Box::new(|st| st.summary(month, 10).unwrap().totals.len()),
        ),
    ];
    let mut worst = 0.0f64;
    for (name, op) in &ops {
        let (m, w, n) = time(RUNS, || op(&st));
        worst = worst.max(m);
        println!("{name:<44} median {m:>7.2} ms   max {w:>7.2} ms   ({n} results)");
    }
    println!("slowest median: {worst:.2} ms (target < 50 ms)");
    drop(st);
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `recap_get` on a real database, opened read-only: the longest available
/// recap, median of 5. Run with
///
///     WRAPPED_PERF_DB=<path> cargo test -p wrapped-core --release --test stats_perf recap_on_db -- --ignored --nocapture
#[test]
#[ignore]
fn recap_on_db() {
    let Some(path) = std::env::var_os("WRAPPED_PERF_DB") else {
        println!("WRAPPED_PERF_DB not set");
        return;
    };
    let s = Store::open_read_only(std::path::Path::new(&path)).unwrap();
    let settings = s.load_settings().unwrap().0;
    let all = collectors_all();
    let st = Stats::new(&s, &all, &settings);
    let today = chrono::Local::now().date_naive();
    let (m, w, list) = time(5, || recap::list(&st, today).unwrap());
    println!(
        "recap list: {} recaps, median {m:.2} ms, max {w:.2} ms",
        list.len()
    );
    let mut candidates: Vec<recap::Period> = list.iter().map(|e| e.period).collect();
    // The current year as well, even before December (the longest range there is).
    candidates.push(recap::Period::containing(recap::PeriodKind::Year, today));
    for p in candidates {
        let (m, w, r) = time(5, || recap::build(&st, p, today).unwrap().unwrap());
        println!(
            "{:<20} {:>4} days {:>2} slides  median {m:>7.2} ms   max {w:>7.2} ms",
            p.id(),
            r.range.days(),
            r.slides.len()
        );
    }
}

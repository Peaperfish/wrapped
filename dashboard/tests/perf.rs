//! The dashboard commands' cost over a synthetic year (the M4 benchmark
//! data: every collector, 365 days). Ignored by default (a measurement, not
//! a check); run with:
//!
//!     cargo test -p wrapped-dashboard --release --test perf -- --ignored --nocapture
//!
//! Each call includes opening and closing the database read-only, as the
//! real commands do. `WRAPPED_PERF_DB=<path>` keeps the database there (to
//! open it in the dashboard with `--db`).

#[path = "../../crates/core/tests/common/mod.rs"]
mod common;

use std::time::Instant;

use common::*;
use wrapped_core::Store;
use wrapped_dashboard::data;

/// A named, timed command call.
type Op<'a> = (&'static str, Box<dyn Fn() + 'a>);

fn time<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, f64) {
    let mut ms = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        std::hint::black_box(f());
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    (ms[ms.len() / 2], ms[ms.len() - 1])
}

#[test]
#[ignore]
fn commands_over_a_synthetic_year() {
    let dir = std::env::temp_dir().join(format!("wrapped-dashboard-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::env::var_os("WRAPPED_PERF_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir.join("wrapped.db"));
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
    let mb = std::fs::metadata(&path).unwrap().len() as f64 / 1e6;
    println!("synthetic year: {mb:.1} MB at {}", path.display());
    let today = d("2026-11-16");
    let rows: Vec<Op<'_>> = vec![
        (
            "meta",
            Box::new(|| drop(data::meta(Some(&path), today).unwrap())),
        ),
        (
            "summary, full year (2025-11-17..2026-11-16)",
            Box::new(|| drop(data::summary(&path, "2025-11-17", "2026-11-16").unwrap())),
        ),
        (
            "summary, calendar year 2026",
            Box::new(|| drop(data::summary(&path, "2026-01-01", "2026-12-31").unwrap())),
        ),
        (
            "summary, one month",
            Box::new(|| drop(data::summary(&path, "2026-10-01", "2026-10-31").unwrap())),
        ),
        (
            "series 2026, screen time",
            Box::new(|| drop(data::series(&path, 2026, "app_usage/screen_time_secs").unwrap())),
        ),
        (
            "series 2026, bytes written",
            Box::new(|| drop(data::series(&path, 2026, "disk_memory/bytes_written").unwrap())),
        ),
    ];
    println!("{:<46} {:>9} {:>9}", "command (7 runs)", "median", "max");
    for (name, f) in &rows {
        let (med, max) = time(7, f);
        println!("{name:<46} {med:>6.2} ms {max:>6.2} ms");
    }
    // For comparison: the same summary on one connection kept open (what the
    // dashboard deliberately does not do, to never hold back checkpoints).
    let store = Store::open_read_only(&path).unwrap();
    let settings = store.load_settings().unwrap().0;
    let all = wrapped_core::collectors::builtin();
    let st = wrapped_core::stats::Stats::new(&store, &all, &settings);
    let r = wrapped_core::stats::DateRange::new(d("2025-11-17"), d("2026-11-16")).unwrap();
    let (med, max) = time(7, || st.summary(r, 5).unwrap().to_json());
    let name = "summary, full year, connection kept open";
    println!("{name:<46} {med:>6.2} ms {max:>6.2} ms");
    drop(st);
    drop(store);
    if std::env::var_os("WRAPPED_PERF_DB").is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

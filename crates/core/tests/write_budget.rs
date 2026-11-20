//! Measures how many bytes SQLite writes for one simulated day of use.
//! Ignored by default (it is a measurement, not a check); run with:
//!
//!     cargo test -p wrapped-core --release --test write_budget -- --ignored --nocapture
//!
//! Simulated day: the front app changes ~1,050 times a day (the measured real
//! rate, spread evenly over the run) among 15 apps, a
//! 20-minute idle break every 4 hours, flushes on the normal 10-minute policy,
//! then a full checkpoint. Four runs: 16 h and 24 h awake, each without and
//! with the Disk & Memory collector (fake counters; 192 / 288 samples).
//!
//! Reported separately:
//! * WAL bytes: everything appended to the WAL during the day (auto-checkpoint
//!   is disabled for the run, so the WAL is never reset and its size is the
//!   total appended). Schema creation is checkpointed away before the day starts.
//! * Checkpoint bytes: what a full checkpoint copies back into the main
//!   database file. A checkpoint writes the latest copy of each distinct page
//!   once, so this is (distinct page numbers in the WAL) x page size, parsed
//!   from the WAL frame headers just before checkpointing.
//! * Total = WAL + checkpoint. With the default auto-checkpoint (1000 pages) a
//!   checkpoint happens every few days instead of daily, so the daily-checkpoint
//!   figure here is the higher of the two.
//!
//! Not counted: filesystem metadata, APFS copy-on-write overhead and the `-shm`
//! index (a memory-mapped file). On macOS the kernel's per-process physical
//! write counter is also printed as an independent cross-check.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use wrapped_core::probe::{DiskMemoryReading, PressureLevel, SystemProbe};
use wrapped_core::{Engine, ManualClock, Store, SystemEvent, collectors};

/// App switches in a simulated day (the user's measured average).
const SWITCHES_PER_DAY: i64 = 1050;

/// Counters that grow by a varying amount on every read.
struct GrowingCounters(Mutex<DiskMemoryReading>);

impl SystemProbe for GrowingCounters {
    fn disk_memory(&self) -> Option<DiskMemoryReading> {
        let mut r = self.0.lock().unwrap();
        let n = r.disk_bytes_written / 4096 % 97;
        r.disk_bytes_read += 3_000_000 + n * 40_960;
        r.disk_bytes_written += 1_500_000 + n * 4_096;
        r.swap_out_bytes += n * 16_384 % 3;
        r.compressed_bytes = 1_000_000_000 + n * 1_000_000;
        r.pressure = if n.is_multiple_of(13) {
            PressureLevel::Warning
        } else {
            PressureLevel::Normal
        };
        Some(*r)
    }
}

#[cfg(target_os = "macos")]
fn process_disk_bytes_written() -> Option<u64> {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        )
    };
    (rc == 0).then_some(info.ri_diskio_byteswritten)
}

#[cfg(not(target_os = "macos"))]
fn process_disk_bytes_written() -> Option<u64> {
    None
}

/// (frames, distinct pages, page size) of a WAL file.
fn wal_pages(wal: &Path) -> (usize, usize, usize) {
    let data = std::fs::read(wal).unwrap_or_default();
    if data.len() < 32 {
        return (0, 0, 0);
    }
    let page_size = u32::from_be_bytes(data[8..12].try_into().unwrap()) as usize;
    let mut pages = HashSet::new();
    let mut frames = 0;
    let mut off = 32;
    while off + 24 + page_size <= data.len() {
        pages.insert(u32::from_be_bytes(data[off..off + 4].try_into().unwrap()));
        frames += 1;
        off += 24 + page_size;
    }
    (frames, pages.len(), page_size)
}

struct Run {
    flushes: u32,
    sessions: i64,
    samples: i64,
    wal_bytes: u64,
    frames: usize,
    checkpoint_bytes: u64,
    distinct: usize,
    kernel: Option<u64>,
    /// Main database file growth over the day, after the final checkpoint.
    db_growth: u64,
}

fn simulate(hours: i64, disk_memory: bool) -> Run {
    let dir = std::env::temp_dir().join(format!(
        "wrapped-write-budget-{}-{hours}-{disk_memory}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("wrapped.db");
    let wal = dir.join("wrapped.db-wal");
    let start_hour = if hours >= 24 { 0 } else { 7 };
    let c = ManualClock::at_local(2026, 11, 16, start_hour, 0, 0, 480);

    let store = Store::open(&path).unwrap();
    let checkpoint = |s: &Store| {
        s.conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap()
    };
    store
        .conn()
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    if disk_memory {
        store
            .set_setting("collector.disk_memory.enabled", "true")
            .unwrap();
    }
    let mut e = Engine::new(store, Arc::new(c.clone()), collectors::builtin_hermetic()).unwrap();
    e.set_probe(Arc::new(GrowingCounters(Mutex::new(DiskMemoryReading {
        disk_only: false,
        boot_time_ms: 1_790_000_000_000,
        disk_bytes_read: 10_000_000_000,
        disk_bytes_written: 5_000_000_000,
        swap_in_bytes: 0,
        swap_out_bytes: 0,
        pressure: PressureLevel::Normal,
        compressed_bytes: 1_000_000_000,
    }))));
    checkpoint(e.store()); // exclude schema creation from the day's numbers
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    assert_eq!(size(&wal), 0);
    let db_before = size(&path);
    let proc_before = process_disk_bytes_written();

    let mut flushes = 0u32;
    let mut switches = 0i64;
    let mut minute = 0i64;
    while minute < hours * 60 {
        // Switches due by the end of this minute minus those already done.
        let due = (minute + 1) * SWITCHES_PER_DAY / (hours * 60);
        while switches < due {
            e.handle(SystemEvent::FrontApp {
                bundle_id: format!("com.example.app{}", switches % 15),
                name: "App".into(),
            })
            .unwrap();
            switches += 1;
        }
        c.advance_secs(60);
        minute += 1;
        let idle = minute % 240 >= 220; // 20-min break every 4 h
        let secs = if idle {
            f64::from(((minute % 240) - 219) as i32 * 60)
        } else {
            5.0
        };
        if e.handle(SystemEvent::IdleTick {
            secs_since_input: secs,
            display_assertions: vec![],
        })
        .unwrap()
        .is_some()
        {
            flushes += 1;
        }
    }
    e.handle(SystemEvent::WillSleep).unwrap();
    flushes += 1;

    let wal_bytes = size(&wal);
    let (frames, distinct, page_size) = wal_pages(&wal);
    let checkpoint_bytes = (distinct * page_size) as u64;
    checkpoint(e.store());
    let proc_after = process_disk_bytes_written();
    let count = |sql: &str| -> i64 { e.store().conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    let run = Run {
        flushes,
        sessions: count("SELECT count(*) FROM sessions"),
        samples: count("SELECT count(DISTINCT time_ms) FROM samples"),
        wal_bytes,
        frames,
        checkpoint_bytes,
        distinct,
        kernel: proc_before.zip(proc_after).map(|(a, b)| b - a),
        db_growth: size(&path).saturating_sub(db_before),
    };
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
    run
}

#[test]
#[ignore]
fn one_simulated_day() {
    let mb = |b: u64| b as f64 / 1_048_576.0;
    println!(
        "{:<26} {:>7} {:>8} {:>13} {:>11} {:>13} {:>13} {:>13} {:>16}",
        "run",
        "flushes",
        "samples",
        "WAL bytes",
        "frames/fl",
        "checkpoint",
        "total",
        "kernel",
        "DB growth/year"
    );
    for (hours, dm) in [(16, false), (16, true), (24, false), (24, true)] {
        let r = simulate(hours, dm);
        let total = r.wal_bytes + r.checkpoint_bytes;
        println!(
            "{:<26} {:>7} {:>8} {:>13} {:>11.1} {:>13} {:>13} {:>13} {:>16}",
            format!(
                "{hours} h awake, disk_memory {}",
                if dm { "on" } else { "off" }
            ),
            r.flushes,
            r.samples,
            r.wal_bytes,
            r.frames as f64 / f64::from(r.flushes),
            format!("{} ({}p)", r.checkpoint_bytes, r.distinct),
            format!("{total} ({:.2} MB)", mb(total)),
            r.kernel.map_or("n/a".to_string(), |k| format!("{k}")),
            format!("{:.1} MB", mb(r.db_growth * 365)),
        );
        assert!(r.sessions > 0);
    }
}

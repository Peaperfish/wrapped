//! Measures how many bytes SQLite writes for one simulated day of use.
//! Ignored by default (it is a measurement, not a check); run with:
//!
//!     cargo test -p wrapped-core --release --test write_budget -- --ignored --nocapture
//!
//! Simulated day: 16 h awake, the front app changes every 3 minutes among 15
//! apps, a 20-minute idle break every 4 hours, flushes on the normal 10-minute
//! policy, then a full checkpoint.
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
use std::sync::Arc;

use wrapped_core::{Engine, ManualClock, Store, SystemEvent, collectors};

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

#[test]
#[ignore]
fn one_simulated_day() {
    let dir = std::env::temp_dir().join(format!("wrapped-write-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("wrapped.db");
    let wal = dir.join("wrapped.db-wal");
    let c = ManualClock::at_local(2026, 11, 16, 7, 0, 0, 480);

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
    let mut e = Engine::new(store, Arc::new(c.clone()), collectors::builtin()).unwrap();
    checkpoint(e.store()); // exclude schema creation from the day's numbers
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    assert_eq!(size(&wal), 0);
    let proc_before = process_disk_bytes_written();

    let mut flushes = 0u32;
    let mut minute = 0i64;
    while minute < 16 * 60 {
        if minute % 3 == 0 {
            e.handle(SystemEvent::FrontApp {
                bundle_id: format!("com.example.app{}", (minute / 3) % 15),
                name: "App".into(),
            })
            .unwrap();
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
            display_assertion_holders: vec![],
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
    let sessions: i64 = e
        .store()
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    let mb = |b: u64| b as f64 / 1_048_576.0;

    println!("flushes:              {flushes}");
    println!("sessions stored:      {sessions}");
    println!(
        "WAL bytes:            {wal_bytes} ({:.2} MB; {frames} frames, {:.1} frames/flush)",
        mb(wal_bytes),
        frames as f64 / f64::from(flushes)
    );
    println!(
        "checkpoint bytes:     {checkpoint_bytes} ({:.2} MB; {distinct} distinct pages x {page_size})",
        mb(checkpoint_bytes)
    );
    println!(
        "total:                {} ({:.2} MB/day)",
        wal_bytes + checkpoint_bytes,
        mb(wal_bytes + checkpoint_bytes)
    );
    match (proc_before, proc_after) {
        (Some(a), Some(b)) => println!(
            "kernel ri_diskio_byteswritten delta for this test process: {} ({:.2} MB)",
            b - a,
            mb(b - a)
        ),
        _ => println!("kernel per-process write counter: not available on this OS"),
    }
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}

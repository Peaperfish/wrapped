//! Measures how many bytes SQLite writes for one simulated day of use.
//! Ignored by default (it is a measurement, not a check); run with:
//!
//!     cargo test -p wrapped-core --release --test write_budget -- --ignored --nocapture
//!
//! Simulated day: 16 h awake, the front app changes every 3 minutes among 15
//! apps, a 20-minute idle break every 4 hours, flushes on the normal 10-minute
//! policy. Auto-checkpointing is disabled during the run so the WAL file size
//! equals every byte SQLite appended; a final checkpoint then shows what the
//! main database file grew by. This counts SQLite's own writes only, not
//! filesystem metadata or APFS copy-on-write overhead.

use std::sync::Arc;

use wrapped_core::{Engine, ManualClock, Store, SystemEvent, collectors};

#[test]
#[ignore]
fn one_simulated_day() {
    let dir = std::env::temp_dir().join(format!("wrapped-write-budget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("wrapped.db");
    let c = ManualClock::at_local(2026, 11, 16, 7, 0, 0, 480);

    let store = Store::open(&path).unwrap();
    store
        .conn()
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    let mut e = Engine::new(store, Arc::new(c.clone()), collectors::builtin()).unwrap();
    let size = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let wal = dir.join("wrapped.db-wal");
    let db_before = size(&path);

    let mut flushes = 0;
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
    // WAL = 32-byte header + frames of (24-byte header + 4096-byte page).
    let frames = wal_bytes.saturating_sub(32) / (24 + 4096);
    e.store()
        .conn()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    let db_after = size(&path);
    let sessions: i64 = e
        .store()
        .conn()
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    println!("flushes:                {flushes}");
    println!("sessions stored:        {sessions}");
    println!(
        "WAL bytes appended:     {wal_bytes} ({frames} pages, {:.1} pages/flush)",
        frames as f64 / f64::from(flushes)
    );
    println!(
        "main db file growth:    {} bytes (written at checkpoint)",
        db_after - db_before
    );
    println!(
        "total SQLite writes:    {:.2} MB/day (WAL + checkpoint upper bound {:.2} MB)",
        wal_bytes as f64 / 1_048_576.0,
        (wal_bytes + db_after) as f64 / 1_048_576.0
    );
    drop(e);
    let _ = std::fs::remove_dir_all(&dir);
}

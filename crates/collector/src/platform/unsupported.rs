//! Placeholder for platforms without a collector yet.
//! It exists so the workspace builds and the core tests run everywhere; the
//! CLI's database commands still work with `WRAPPED_DB` set, and
//! `wrapped-collector dashboard` starts the dashboard next to it.

use std::path::{Path, PathBuf};

pub fn data_dir() -> Option<PathBuf> {
    None
}

pub fn run_collector(_db: &Path, _debug: bool) -> i32 {
    eprintln!("wrapped-collector: no collector for this platform yet");
    1
}

pub fn running_pid(_db: &Path) -> Option<i32> {
    None
}

pub fn request_flush(_db: &Path) -> Option<i32> {
    None
}

/// No collector runs on this platform.
pub fn wake_for_ctl(_db: &Path) -> bool {
    false
}

/// Start the dashboard (`wrapped-dashboard[.exe]` next to this executable)
/// on `db`. No single-instance handling yet: a second call opens a second
/// window.
pub fn open_dashboard(db: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;
    let bin = exe.with_file_name(format!("wrapped-dashboard{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        return Err(format!("no dashboard at {}", bin.display()));
    }
    let child = std::process::Command::new(&bin)
        .arg("--db")
        .arg(db)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    // Reap it when its window closes.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

pub fn login(_cmd: &str) -> i32 {
    eprintln!("wrapped-collector: launch at login is not supported on this platform yet");
    1
}

pub fn scan_hooks() -> wrapped_core::collector::ScanHooks {
    wrapped_core::collector::ScanHooks::default()
}

pub struct Usage {
    pub cpu_secs: f64,
    pub disk_bytes_read: u64,
    pub disk_bytes_written: u64,
    pub peak_footprint: u64,
}

pub fn usage() -> Option<Usage> {
    None
}

pub fn children_cpu_secs() -> f64 {
    0.0
}

/// Same shape as the macOS guard. No single-instance lock exists on this
/// platform yet (no collector can run here), so there is nothing to hold.
pub struct DbLock;

// Empty for now (the real lock comes in M9); it keeps code written against the
// macOS lock, which has a Drop, passing clippy here too.
impl Drop for DbLock {
    fn drop(&mut self) {}
}

pub fn hold_db_lock(_db: &Path) -> Result<DbLock, String> {
    Ok(DbLock)
}

/// No disk / memory counters on this platform yet: the Disk & Memory
/// collector records nothing.
pub fn disk_memory() -> Option<wrapped_core::probe::DiskMemoryReading> {
    None
}

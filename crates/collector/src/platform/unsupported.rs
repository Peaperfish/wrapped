//! Placeholder for platforms without a collector yet (Windows: Milestone 9).
//! It exists so the workspace builds and the core tests run everywhere; the
//! CLI's database commands still work with `WRAPPED_DB` set.

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

pub fn hold_db_lock(_db: &Path) -> Result<DbLock, String> {
    Ok(DbLock)
}

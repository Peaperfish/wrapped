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

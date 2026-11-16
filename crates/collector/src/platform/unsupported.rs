//! Placeholder for platforms without a collector yet (Windows: Milestone 9).
//! It exists so the workspace builds and the core tests run everywhere.

use std::path::PathBuf;

pub fn data_dir() -> Option<PathBuf> {
    None
}

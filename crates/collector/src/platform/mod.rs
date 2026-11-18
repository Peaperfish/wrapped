//! Platform layer: everything OS-specific lives below this module.
//!
//! Each platform provides the same functions (macOS now, Windows in
//! Milestone 9):
//! * `data_dir()`: where `wrapped.db` lives.
//! * `run_collector(db, debug)`: the tray app. Turns OS signals into
//!   [`wrapped_core::SystemEvent`]s and feeds them to the engine on one thread.
//! * `running_pid(db)` / `request_flush(db)`: find / ask a running collector
//!   to flush now.
//! * `login(cmd)`: launch-at-login `enable` / `disable` / `status`.
//! * `scan_hooks()`: lowest priority for scan threads and their child processes.
//! * `hold_db_lock(db)`: single-writer lock for a CLI scan.
//! * `usage()` / `children_cpu_secs()`: resource usage for the scan report.
//!
//! Nothing here makes tracking decisions: it only translates OS signals.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
pub use unsupported::*;

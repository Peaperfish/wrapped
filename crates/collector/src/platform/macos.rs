//! macOS platform layer. The event source (NSWorkspace notifications, idle
//! tick, tray) is ported from the M0 spike in Phase B.

use std::path::PathBuf;

/// `~/Library/Application Support/Wrapped`
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Wrapped"))
}

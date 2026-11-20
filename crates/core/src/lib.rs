//! Wrapped core: everything that is not platform code.
//!
//! * [`ai`]: AI usage: adapters for the tools' logs, sources, the scan driver.
//! * [`event`]: the system event vocabulary the platform layer produces.
//! * [`collector`]: the plugin trait, the [`collector::Sink`] and buffer.
//! * [`presence`]: the shared idle / sleep / lock state machine.
//! * [`now`]: live hardware numbers for the Now tab (a sampler that runs only when called).
//! * [`probe`]: system counters (disk, memory) read through the platform layer.
//! * [`collectors`]: the built-in collectors and their registration list.
//! * [`ctl`]: the request-file channel from the dashboard / CLI to the collector.
//! * [`scan`]: folder walk, hashing and bounded EXIF reading for scanned collectors.
//! * [`store`]: SQLite schema, migrations, batched writes and daily rollups.
//! * [`engine`]: routes events, owns the flush policy, pause and settings.
//! * [`recap`]: weekly / monthly / yearly recap slides from the stats engine.
//! * [`stats`]: the stats engine: totals, series, top-N, comparisons and
//!   records for any range of local dates (dashboard, recaps).

pub mod ai;
pub mod collector;
pub mod collectors;
pub mod ctl;
pub mod engine;
pub mod event;
pub mod now;
pub mod presence;
pub mod probe;
pub mod recap;
pub mod scan;
pub mod settings;
pub mod stats;
pub mod store;
pub mod time;

pub use collector::{Collector, CollectorInfo, Kind, MetricDef, Sink};
pub use engine::Engine;
pub use event::{DisplayAssertion, SystemEvent};
pub use settings::{Pause, Settings};
pub use store::Store;
pub use time::{Clock, ManualClock, SystemClock, Timestamp};

/// Windows clients and the writer share the current user's local app data.
#[cfg(windows)]
pub fn windows_data_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|p| !p.is_empty())
        .map(|p| std::path::PathBuf::from(p).join("Wrapped"))
}

/// Spawn a helper without allocating a Windows console for a GUI parent.
pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new(program);
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        command
    }
    #[cfg(not(windows))]
    std::process::Command::new(program)
}

#[cfg(windows)]
pub mod windows_ipc;

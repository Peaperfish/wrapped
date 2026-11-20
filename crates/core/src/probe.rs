//! System-wide counters read on demand (Disk & Memory collector).
//!
//! The platform layer implements [`SystemProbe`]; core only sees the plain
//! [`DiskMemoryReading`] it returns, so the collector stays OS-agnostic and is
//! tested with fake readings. A platform without an implementation returns
//! `None` and the collector records nothing.

/// The OS memory pressure level at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureLevel {
    Normal,
    Warning,
    Critical,
}

impl PressureLevel {
    pub const ALL: [PressureLevel; 3] = [Self::Normal, Self::Warning, Self::Critical];

    /// `daily_metrics` key.
    pub fn key(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.key() == key)
    }
}

/// One reading of the disk and memory counters.
///
/// `disk_*` and `swap_*` are cumulative since boot (they restart from zero on
/// reboot); `pressure` and `compressed_bytes` are the state right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskMemoryReading {
    /// This source only exposes physical disk totals.
    pub disk_only: bool,
    /// When the system booted (UTC ms). A different value means the
    /// cumulative counters restarted.
    pub boot_time_ms: i64,
    /// Bytes read from internal storage since boot (all internal physical
    /// disks summed; disk images and removable media excluded).
    pub disk_bytes_read: u64,
    /// Bytes written to internal storage since boot.
    pub disk_bytes_written: u64,
    /// Bytes swapped in from the swap files since boot.
    pub swap_in_bytes: u64,
    /// Bytes swapped out to the swap files since boot.
    pub swap_out_bytes: u64,
    pub pressure: PressureLevel,
    /// Physical memory occupied by the memory compressor right now.
    pub compressed_bytes: u64,
}

/// Reads system counters. Implemented by the platform layer.
pub trait SystemProbe: Send + Sync {
    /// `None` when the platform cannot read them (or a read failed).
    fn disk_memory(&self) -> Option<DiskMemoryReading>;
}

/// A probe that never has readings (the default until the platform sets one).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoProbe;

impl SystemProbe for NoProbe {
    fn disk_memory(&self) -> Option<DiskMemoryReading> {
        None
    }
}

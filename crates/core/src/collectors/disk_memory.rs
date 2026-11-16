//! Disk & Memory (DESIGN.md §7): system-wide disk bytes read / written, swap
//! in / out, memory pressure and compressed memory. Live, off by default.
//!
//! Every 5 minutes of wall-clock time (on the first minute tick at least
//! [`SAMPLE_MIN_GAP_MS`] after the previous reading), while the system is awake
//! and tracking is not paused, the collector reads the counters through the
//! platform's [`SystemProbe`](crate::probe::SystemProbe) and records one sample:
//! the deltas since the previous reading, the pressure level and compressed
//! memory at that moment, and how many seconds of awake time it covers.
//!
//! * **Sleep.** Dark-wake ticks (between `WillSleep` and `DidWake`, the
//!   engine's asleep state) never read. The first awake tick after a wake
//!   records one delta covering everything since the last reading: the I/O
//!   done in the last minutes before sleep and during dark wakes lands on the
//!   wake tick; it is not spread over the night. The sample's `secs` counts
//!   only awake time (before the sleep plus since the wake).
//! * **Resets.** Cumulative counters restart at boot. A reading with a
//!   different boot time, a counter lower than before, or an impossible rate
//!   re-baselines: no sample for that interval (never a negative or a huge
//!   delta). A wall clock moving backwards re-baselines too.
//! * **Pause** drops the baseline; the first tick after resuming takes a new
//!   one, so nothing done while paused is recorded.
//!
//! A sample at `t` with `secs = s` describes the window `[t - s, t]` (local
//! time in the sample's own offset). Daily metrics split that window across
//! local days and hours in proportion to time; pressure seconds credit the
//! level seen at `t` to the window.

use std::collections::BTreeMap;

use crate::collector::{
    Collector, CollectorInfo, DayInput, DaySample, Kind, MAX_SAMPLE_WINDOW_MS, MetricDef,
    MetricValue, Sink, Unit,
};
use crate::event::SystemEvent;
use crate::probe::{DiskMemoryReading, PressureLevel};
use crate::time::{MS_PER_DAY, MS_PER_HOUR, MS_PER_MIN, MS_PER_SEC, Timestamp, local_midnight_ms};

pub const NAME: &str = "disk_memory";

/// Nominal time between readings.
pub const SAMPLE_EVERY_MS: i64 = 5 * MS_PER_MIN;
/// Ticks come about every 60 s (±5 s tolerance), so "5 minutes" is the first
/// tick at least 4.5 min after the previous reading.
pub const SAMPLE_MIN_GAP_MS: i64 = SAMPLE_EVERY_MS - 30 * MS_PER_SEC;
/// Boot times further apart than this mean a reboot (the reported boot time
/// can shift slightly when the wall clock is adjusted).
pub const BOOT_TOLERANCE_MS: i64 = MS_PER_SEC;
/// Faster than any internal SSD or swap: a larger delta is a counter reset.
pub const MAX_BYTES_PER_SEC: u64 = 32 * 1024 * 1024 * 1024;

/// The sample metric: one row per reading. `value` is the awake seconds the
/// reading covers; `data` holds the rest with short keys (bytes; see [`key`]),
/// e.g. `{"r":48066560,"w":30089216,"c":1428439040,"p":"normal"}`. Swap keys
/// are left out when zero. Short keys keep rows small: fewer B-tree page
/// splits per day (see DEVIATIONS.md, write budget).
pub const SAMPLE_METRIC: &str = "reading";

/// Keys of a reading's `data` object.
pub mod key {
    /// Disk bytes read in the window.
    pub const READ: &str = "r";
    /// Disk bytes written in the window.
    pub const WRITTEN: &str = "w";
    /// Swap-in bytes in the window (absent = 0).
    pub const SWAP_IN: &str = "si";
    /// Swap-out bytes in the window (absent = 0).
    pub const SWAP_OUT: &str = "so";
    /// Compressed memory bytes at the reading.
    pub const COMPRESSED: &str = "c";
    /// Pressure level at the reading: normal | warning | critical.
    pub const PRESSURE: &str = "p";
}

/// One recorded reading: the deltas since the previous one and the state now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    pub time: Timestamp,
    /// Awake seconds this sample covers (the window `[time - secs, time]`).
    pub secs: f64,
    pub bytes_read: f64,
    pub bytes_written: f64,
    pub swap_in: f64,
    pub swap_out: f64,
    pub pressure: PressureLevel,
    pub compressed: f64,
}

impl Tick {
    fn emit(&self, out: &Sink) {
        let mut data = serde_json::Map::new();
        let mut put = |k: &str, v: f64| {
            // Zero swap is the common case: leave it out (absent = 0).
            if v != 0.0 || !k.starts_with('s') {
                data.insert(k.into(), serde_json::json!(v));
            }
        };
        put(key::READ, self.bytes_read);
        put(key::WRITTEN, self.bytes_written);
        put(key::SWAP_IN, self.swap_in);
        put(key::SWAP_OUT, self.swap_out);
        put(key::COMPRESSED, self.compressed);
        data.insert(key::PRESSURE.into(), self.pressure.key().into());
        out.sample_with(
            SAMPLE_METRIC,
            self.time,
            self.secs,
            serde_json::Value::Object(data).to_string(),
        );
    }

    /// Reassemble ticks from stored sample rows. Rows of other metrics, or
    /// without a known pressure level, are ignored.
    pub fn from_samples(samples: &[DaySample]) -> Vec<Tick> {
        samples
            .iter()
            .filter(|s| s.metric == SAMPLE_METRIC)
            .filter_map(|s| {
                let d = &s.data;
                let g = |k: &str| d.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
                let pressure = PressureLevel::from_key(d.get(key::PRESSURE)?.as_str()?)?;
                Some(Tick {
                    time: s.time,
                    secs: s.value,
                    bytes_read: g(key::READ),
                    bytes_written: g(key::WRITTEN),
                    swap_in: g(key::SWAP_IN),
                    swap_out: g(key::SWAP_OUT),
                    pressure,
                    compressed: g(key::COMPRESSED),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct Base {
    reading: DiskMemoryReading,
    at: Timestamp,
}

#[derive(Default)]
pub struct DiskMemory {
    out: Option<Sink>,
    paused: bool,
    /// The previous reading (the baseline for the next deltas).
    base: Option<Base>,
    /// Awake ms since the baseline in stretches already closed by a sleep.
    awake_ms: i64,
    /// Start of the current awake stretch; `None` while asleep.
    awake_from: Option<Timestamp>,
}

/// Counter deltas `cur - prev`, or `None` if the counters restarted (other
/// boot, a counter went down, or an impossible rate over `wall_ms`).
pub fn deltas(prev: &DiskMemoryReading, cur: &DiskMemoryReading, wall_ms: i64) -> Option<[u64; 4]> {
    if (cur.boot_time_ms - prev.boot_time_ms).abs() > BOOT_TOLERANCE_MS {
        return None;
    }
    let d = [
        cur.disk_bytes_read.checked_sub(prev.disk_bytes_read)?,
        cur.disk_bytes_written.checked_sub(prev.disk_bytes_written)?,
        cur.swap_in_bytes.checked_sub(prev.swap_in_bytes)?,
        cur.swap_out_bytes.checked_sub(prev.swap_out_bytes)?,
    ];
    let secs = (wall_ms.max(0) / MS_PER_SEC).max(1) as u64;
    let cap = MAX_BYTES_PER_SEC.saturating_mul(secs);
    d.iter().all(|v| *v <= cap).then_some(d)
}

impl DiskMemory {
    fn rebaseline(&mut self, out: &Sink, now: Timestamp) {
        self.base = out
            .read_disk_memory()
            .map(|reading| Base { reading, at: now });
        self.awake_ms = 0;
        self.awake_from = Some(now);
    }

    fn close_awake_stretch(&mut self, now: Timestamp) {
        if let Some(from) = self.awake_from.take() {
            self.awake_ms += (now.utc_ms - from.utc_ms).max(0);
        }
    }

    fn on_tick(&mut self, out: &Sink) {
        if out.system_asleep() {
            return; // dark wake: never read
        }
        let now = out.now();
        if self.awake_from.is_none() {
            // Awake again without a DidWake (missed notification): count from here.
            self.awake_from = Some(now);
        }
        if self.paused {
            return;
        }
        let Some(base) = self.base else {
            self.rebaseline(out, now);
            return;
        };
        let wall = now.utc_ms - base.at.utc_ms;
        if wall < 0 {
            self.rebaseline(out, now); // wall clock moved backwards
            return;
        }
        if wall < SAMPLE_MIN_GAP_MS {
            return;
        }
        let Some(cur) = out.read_disk_memory() else {
            return; // read failed: keep the baseline, try again next tick
        };
        let from = self.awake_from.map_or(now.utc_ms, |f| f.utc_ms);
        let awake = (self.awake_ms + (now.utc_ms - from).max(0))
            .min(wall)
            .min(MAX_SAMPLE_WINDOW_MS);
        if let Some([read, written, swap_in, swap_out]) = deltas(&base.reading, &cur, wall) {
            Tick {
                time: now,
                secs: awake as f64 / MS_PER_SEC as f64,
                bytes_read: read as f64,
                bytes_written: written as f64,
                swap_in: swap_in as f64,
                swap_out: swap_out as f64,
                pressure: cur.pressure,
                compressed: cur.compressed_bytes as f64,
            }
            .emit(out);
        }
        self.base = Some(Base {
            reading: cur,
            at: now,
        });
        self.awake_ms = 0;
        self.awake_from = Some(now);
    }
}

impl Collector for DiskMemory {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "Disk & Memory",
            description: "System-wide disk bytes read and written, swap in and out, memory \
                          pressure and compressed memory, sampled every 5 minutes while awake.",
            permissions: "None",
            cost: "One counter read every 5 minutes while awake; ~0% CPU. Under 1 MB per day \
                   on disk.",
            default_on: false,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Live
    }

    fn start(&mut self, out: Sink) {
        self.out = Some(out);
        self.base = None;
        self.awake_ms = 0;
        self.awake_from = None;
    }

    fn on_event(&mut self, ev: &SystemEvent) {
        let Some(out) = self.out.clone() else { return };
        match ev {
            SystemEvent::Paused => {
                self.paused = true;
                self.base = None;
            }
            SystemEvent::Resumed => self.paused = false,
            SystemEvent::WillSleep | SystemEvent::WillPowerOff => {
                self.close_awake_stretch(out.now());
            }
            SystemEvent::DidWake => {
                if self.awake_from.is_none() {
                    self.awake_from = Some(out.now());
                }
            }
            SystemEvent::IdleTick { .. } => self.on_tick(&out),
            _ => {}
        }
    }

    fn on_data_deleted(&mut self) {
        // The next reading must not carry pre-delete time.
        if let Some(out) = self.out.clone()
            && self.base.is_some()
        {
            self.rebaseline(&out, out.now());
        }
    }

    fn stop(&mut self) {
        self.out = None;
        self.base = None;
    }

    fn metrics(&self) -> Vec<MetricDef> {
        let m = |id, title, unit, key| MetricDef {
            id,
            title,
            unit,
            key,
        };
        vec![
            m("bytes_read", "Disk read", Unit::Bytes, None),
            m("bytes_written", "Disk written", Unit::Bytes, None),
            m("swap_in_bytes", "Swapped in", Unit::Bytes, None),
            m("swap_out_bytes", "Swapped out", Unit::Bytes, None),
            m(
                "pressure_secs_by_level",
                "Memory pressure",
                Unit::Seconds,
                Some("normal | warning | critical"),
            ),
            m("compressed_peak_bytes", "Compressed memory, peak", Unit::Bytes, None),
            m("compressed_avg_bytes", "Compressed memory, average", Unit::Bytes, None),
            m(
                "bytes_written_by_hour",
                "Disk written by hour of day",
                Unit::Bytes,
                Some("local hour 00..23"),
            ),
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        rollup_ticks(&Tick::from_samples(day.samples), local_midnight_ms(day.date))
    }
}

/// Daily metrics for the local day starting at `day0` (local ms) from ticks
/// whose windows may overlap it. Empty if none does.
pub fn rollup_ticks(ticks: &[Tick], day0: i64) -> Vec<MetricValue> {
    let day1 = day0 + MS_PER_DAY;
    let (mut read, mut written, mut swap_in, mut swap_out) = (0.0, 0.0, 0.0, 0.0);
    let mut pressure: BTreeMap<&str, f64> = BTreeMap::new();
    let mut by_hour: BTreeMap<u32, f64> = BTreeMap::new();
    let (mut peak, mut weighted, mut weight, mut sum, mut n) = (0.0f64, 0.0, 0.0, 0.0, 0u32);
    for t in ticks {
        let we = t.time.local_ms();
        let win = (t.secs * MS_PER_SEC as f64).round() as i64;
        let ws = we - win;
        let (cs, ce) = (ws.max(day0), we.min(day1));
        // Share of the sample that falls in this day. A zero-length window
        // (a reading right at a wake) belongs to the instant it was taken.
        let frac = if win <= 0 {
            if (day0..day1).contains(&we) { 1.0 } else { 0.0 }
        } else if ce > cs {
            (ce - cs) as f64 / win as f64
        } else {
            0.0
        };
        if frac <= 0.0 {
            continue;
        }
        let overlap_secs = if win <= 0 {
            0.0
        } else {
            (ce - cs) as f64 / MS_PER_SEC as f64
        };
        read += t.bytes_read * frac;
        written += t.bytes_written * frac;
        swap_in += t.swap_in * frac;
        swap_out += t.swap_out * frac;
        *pressure.entry(t.pressure.key()).or_default() += overlap_secs;
        peak = peak.max(t.compressed);
        weighted += t.compressed * overlap_secs;
        weight += overlap_secs;
        sum += t.compressed;
        n += 1;
        if win <= 0 {
            let h = (we.rem_euclid(MS_PER_DAY) / MS_PER_HOUR) as u32;
            *by_hour.entry(h).or_default() += t.bytes_written;
        } else {
            let mut a = cs;
            while a < ce {
                let next = (a - a.rem_euclid(MS_PER_HOUR) + MS_PER_HOUR).min(ce);
                let h = (a.rem_euclid(MS_PER_DAY) / MS_PER_HOUR) as u32;
                *by_hour.entry(h).or_default() +=
                    t.bytes_written * (next - a) as f64 / win as f64;
                a = next;
            }
        }
    }
    if n == 0 {
        return Vec::new();
    }
    let avg = if weight > 0.0 {
        weighted / weight
    } else {
        sum / f64::from(n)
    };
    let mut out = vec![
        MetricValue::new("bytes_read", "", read),
        MetricValue::new("bytes_written", "", written),
        MetricValue::new("swap_in_bytes", "", swap_in),
        MetricValue::new("swap_out_bytes", "", swap_out),
        MetricValue::new("compressed_peak_bytes", "", peak),
        MetricValue::new("compressed_avg_bytes", "", avg),
    ];
    out.extend(
        pressure
            .into_iter()
            .filter(|(_, v)| *v > 0.0)
            .map(|(k, v)| MetricValue::new("pressure_secs_by_level", k, v)),
    );
    out.extend(
        by_hour
            .into_iter()
            .map(|(h, v)| MetricValue::new("bytes_written_by_hour", format!("{h:02}"), v)),
    );
    out
}

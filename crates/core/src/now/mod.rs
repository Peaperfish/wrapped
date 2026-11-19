//! Live hardware numbers for the dashboard's Now tab.
//!
//! A [`NowSampler`] does work only when [`NowSampler::sample`] is called (no
//! thread, no timer). It remembers the previous raw counters, so rates (CPU %,
//! disk / network / swap speed) come from the delta between two calls; the
//! first call returns every rate as `None`. Every [`NowSnapshot`] field is an
//! `Option`: a metric this machine does not expose is `None` ("—").
//!
//! The raw numbers come from a [`NowSource`]: `macos.rs` on macOS (public APIs
//! only: Mach host calls, sysctl, IOKit, the AppleSMC user client), an empty
//! source elsewhere until the Windows sources arrive. Tests use a fake source.

use std::collections::HashMap;

use serde_json::{Value, json};

#[cfg(target_os = "macos")]
mod macos;

/// Two samples closer than this are not turned into rates (the previous
/// baseline is kept), so a tight loop never divides by a near-zero interval.
pub const MIN_INTERVAL_MS: u64 = 100;

/// Faster than any disk, swap file or network link: a larger delta is a
/// counter glitch, not traffic.
const MAX_BYTES_PER_SEC: f64 = 200e9;

/// Performance or efficiency cluster (Apple Silicon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreKind {
    Performance,
    Efficiency,
}

impl CoreKind {
    fn key(self) -> &'static str {
        match self {
            Self::Performance => "P",
            Self::Efficiency => "E",
        }
    }
}

/// Cumulative scheduler ticks of one logical core (32-bit, they wrap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawCore {
    pub busy: u32,
    pub idle: u32,
    pub kind: Option<CoreKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawMem {
    pub used: u64,
    pub total: u64,
    pub compressed: u64,
    /// 1 normal, 2 warning, 4 critical (the kernel's numbers).
    pub pressure: u8,
}

/// Cumulative bytes of one network interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawIface {
    pub id: u32,
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Temp {
    pub label: String,
    pub celsius: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fan {
    pub name: String,
    pub rpm: f64,
    pub min_rpm: Option<f64>,
    pub max_rpm: Option<f64>,
}

/// One reading of every source; `None` where the source could not read it.
#[derive(Debug, Clone, Default)]
pub struct RawNow {
    /// Monotonic milliseconds (only differences matter).
    pub mono_ms: u64,
    pub cores: Option<Vec<RawCore>>,
    pub gpu_percent: Option<f64>,
    pub mem: Option<RawMem>,
    /// (used, total) bytes of swap files right now.
    pub swap: Option<(u64, u64)>,
    /// Cumulative (in, out) bytes swapped since boot.
    pub swap_io: Option<(u64, u64)>,
    /// Cumulative (read, written) bytes of internal storage.
    pub disk: Option<(u64, u64)>,
    pub net: Option<Vec<RawIface>>,
    pub temps: Option<Vec<Temp>>,
    pub fans: Option<Vec<Fan>>,
}

/// Where the raw numbers come from.
pub trait NowSource: Send {
    /// The chip / CPU model, if known (read once).
    fn chip(&mut self) -> Option<String>;
    fn read(&mut self) -> RawNow;
}

/// A source with nothing to report (platforms without an implementation).
#[derive(Debug, Default)]
pub struct EmptySource {
    start: Option<std::time::Instant>,
}

impl NowSource for EmptySource {
    fn chip(&mut self) -> Option<String> {
        None
    }
    fn read(&mut self) -> RawNow {
        let start = *self.start.get_or_insert_with(std::time::Instant::now);
        RawNow {
            mono_ms: start.elapsed().as_millis() as u64,
            ..RawNow::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreLoad {
    pub percent: Option<f64>,
    pub kind: Option<CoreKind>,
}

/// What the Now tab shows. Rates are per second; sizes are bytes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowSnapshot {
    /// Wall clock, UTC ms.
    pub ts_ms: Option<i64>,
    pub chip: Option<String>,
    pub cpu_percent: Option<f64>,
    /// Per logical core, in the system's core order.
    pub cpu_cores: Option<Vec<CoreLoad>>,
    pub cpu_p_percent: Option<f64>,
    pub cpu_e_percent: Option<f64>,
    pub gpu_percent: Option<f64>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    /// "normal" | "warning" | "critical".
    pub mem_pressure: Option<String>,
    pub mem_compressed_bytes: Option<u64>,
    pub swap_used_bytes: Option<u64>,
    pub swap_total_bytes: Option<u64>,
    pub swap_in_per_sec: Option<f64>,
    pub swap_out_per_sec: Option<f64>,
    pub disk_read_per_sec: Option<f64>,
    pub disk_write_per_sec: Option<f64>,
    pub net_down_per_sec: Option<f64>,
    pub net_up_per_sec: Option<f64>,
    /// The hotter of the CPU core groups.
    pub cpu_temp_c: Option<f64>,
    /// Mean of the GPU sensors.
    pub gpu_temp_c: Option<f64>,
    /// Every clearly labelled sensor (CPU / GPU ones included).
    pub temps: Option<Vec<Temp>>,
    pub fans: Option<Vec<Fan>>,
}

fn num(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

impl NowSnapshot {
    /// The JSON the dashboard command returns and `now --json` prints
    /// (`dashboard::types::NowSnapshotJson` describes it).
    pub fn to_json(&self) -> Value {
        let cores = self.cpu_cores.as_ref().map(|cs| {
            cs.iter()
                .map(|c| json!({"percent": num(c.percent), "kind": c.kind.map(CoreKind::key)}))
                .collect::<Vec<_>>()
        });
        let temps = self.temps.as_ref().map(|ts| {
            ts.iter()
                .map(|t| json!({"label": t.label, "celsius": num(Some(t.celsius))}))
                .collect::<Vec<_>>()
        });
        let fans = self.fans.as_ref().map(|fs| {
            fs.iter()
                .map(|f| {
                    json!({"name": f.name, "rpm": num(Some(f.rpm)),
                           "min_rpm": num(f.min_rpm), "max_rpm": num(f.max_rpm)})
                })
                .collect::<Vec<_>>()
        });
        json!({
            "ts_ms": self.ts_ms,
            "chip": self.chip,
            "cpu_percent": num(self.cpu_percent),
            "cpu_cores": cores,
            "cpu_p_percent": num(self.cpu_p_percent),
            "cpu_e_percent": num(self.cpu_e_percent),
            "gpu_percent": num(self.gpu_percent),
            "mem_used_bytes": self.mem_used_bytes,
            "mem_total_bytes": self.mem_total_bytes,
            "mem_pressure": self.mem_pressure,
            "mem_compressed_bytes": self.mem_compressed_bytes,
            "swap_used_bytes": self.swap_used_bytes,
            "swap_total_bytes": self.swap_total_bytes,
            "swap_in_per_sec": num(self.swap_in_per_sec),
            "swap_out_per_sec": num(self.swap_out_per_sec),
            "disk_read_per_sec": num(self.disk_read_per_sec),
            "disk_write_per_sec": num(self.disk_write_per_sec),
            "net_down_per_sec": num(self.net_down_per_sec),
            "net_up_per_sec": num(self.net_up_per_sec),
            "cpu_temp_c": num(self.cpu_temp_c),
            "gpu_temp_c": num(self.gpu_temp_c),
            "temps": temps,
            "fans": fans,
        })
    }
}

/// Bytes per second between two cumulative counters. `None` if the counter
/// went backwards (reset), the interval is empty, or the result is not
/// physically possible.
pub fn byte_rate(prev: u64, cur: u64, dt_secs: f64) -> Option<f64> {
    if dt_secs <= 0.0 {
        return None;
    }
    let rate = cur.checked_sub(prev)? as f64 / dt_secs;
    (rate <= MAX_BYTES_PER_SEC).then_some(rate)
}

/// Busy fraction (0-100) from two wrapping tick readings; `None` if no ticks
/// passed.
pub fn core_percent(prev: &RawCore, cur: &RawCore) -> Option<f64> {
    let busy = cur.busy.wrapping_sub(prev.busy) as f64;
    let idle = cur.idle.wrapping_sub(prev.idle) as f64;
    let total = busy + idle;
    (total > 0.0).then(|| (busy / total * 100.0).clamp(0.0, 100.0))
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0u32), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

/// Keeps the previous raw reading; see the module docs.
pub struct NowSampler {
    source: Box<dyn NowSource>,
    chip: Option<String>,
    prev: Option<RawNow>,
}

impl NowSampler {
    /// The platform's source.
    pub fn new() -> Self {
        #[cfg(target_os = "macos")]
        let source: Box<dyn NowSource> = Box::new(macos::MacSource::new());
        #[cfg(not(target_os = "macos"))]
        let source: Box<dyn NowSource> = Box::new(EmptySource::default());
        Self::with_source(source)
    }

    pub fn with_source(mut source: Box<dyn NowSource>) -> Self {
        let chip = source.chip();
        Self {
            source,
            chip,
            prev: None,
        }
    }

    /// Reads everything once and returns the snapshot.
    pub fn sample(&mut self) -> NowSnapshot {
        let cur = self.source.read();
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as i64);
        let dt_ms = self
            .prev
            .as_ref()
            .map(|p| cur.mono_ms.saturating_sub(p.mono_ms));
        // Too soon after the baseline: no rates, and the baseline stays.
        let ready = dt_ms.is_some_and(|d| d >= MIN_INTERVAL_MS);
        let basis = match (&self.prev, dt_ms) {
            (Some(p), Some(d)) if ready => Some((p, d as f64 / 1000.0)),
            _ => None,
        };
        let snap = snapshot(self.chip.clone(), wall, &cur, basis);
        if ready || self.prev.is_none() {
            self.prev = Some(cur);
        }
        snap
    }
}

impl Default for NowSampler {
    fn default() -> Self {
        Self::new()
    }
}

fn snapshot(
    chip: Option<String>,
    wall: Option<i64>,
    cur: &RawNow,
    prev: Option<(&RawNow, f64)>,
) -> NowSnapshot {
    let mut s = NowSnapshot {
        ts_ms: wall,
        chip,
        ..NowSnapshot::default()
    };

    // CPU: per core between the two readings (same core count required).
    if let (Some(cur_cores), Some((p, _))) = (&cur.cores, prev)
        && let Some(prev_cores) = p.cores.as_ref().filter(|pc| pc.len() == cur_cores.len())
    {
        let loads: Vec<CoreLoad> = prev_cores
            .iter()
            .zip(cur_cores)
            .map(|(a, b)| CoreLoad {
                percent: core_percent(a, b),
                kind: b.kind,
            })
            .collect();
        let mut sums: HashMap<Option<CoreKind>, (f64, u32)> = HashMap::new();
        let mut all = (0.0, 0u32);
        for l in &loads {
            if let Some(p) = l.percent {
                let e = sums.entry(l.kind).or_default();
                e.0 += p;
                e.1 += 1;
                all.0 += p;
                all.1 += 1;
            }
        }
        let avg = |k: CoreKind| sums.get(&Some(k)).map(|(sum, n)| sum / f64::from(*n));
        s.cpu_p_percent = avg(CoreKind::Performance);
        s.cpu_e_percent = avg(CoreKind::Efficiency);
        // Total = mean over all cores (each core counts the same).
        s.cpu_percent = (all.1 > 0).then(|| all.0 / f64::from(all.1));
        s.cpu_cores = Some(loads);
    } else if let Some(cores) = &cur.cores {
        // First sample: the cores exist but no load is known yet.
        s.cpu_cores = Some(
            cores
                .iter()
                .map(|c| CoreLoad {
                    percent: None,
                    kind: c.kind,
                })
                .collect(),
        );
    }
    s.gpu_percent = cur.gpu_percent.map(|g| g.clamp(0.0, 100.0));

    if let Some(m) = cur.mem {
        s.mem_used_bytes = Some(m.used);
        s.mem_total_bytes = Some(m.total);
        s.mem_compressed_bytes = Some(m.compressed);
        s.mem_pressure = Some(
            match m.pressure {
                4 => "critical",
                2 => "warning",
                _ => "normal",
            }
            .to_string(),
        );
    }
    if let Some((used, total)) = cur.swap {
        s.swap_used_bytes = Some(used);
        s.swap_total_bytes = Some(total);
    }

    if let Some((p, dt)) = prev {
        if let (Some((pi, po)), Some((ci, co))) = (p.swap_io, cur.swap_io) {
            s.swap_in_per_sec = byte_rate(pi, ci, dt);
            s.swap_out_per_sec = byte_rate(po, co, dt);
        }
        if let (Some((pr, pw)), Some((cr, cw))) = (p.disk, cur.disk) {
            s.disk_read_per_sec = byte_rate(pr, cr, dt);
            s.disk_write_per_sec = byte_rate(pw, cw, dt);
        }
        if let (Some(pn), Some(cn)) = (&p.net, &cur.net) {
            // Per interface, so one appearing, vanishing or resetting never
            // makes the total negative or huge.
            let before: HashMap<u32, &RawIface> = pn.iter().map(|i| (i.id, i)).collect();
            let (mut down, mut up) = (0.0, 0.0);
            for i in cn {
                if let Some(b) = before.get(&i.id) {
                    down += byte_rate(b.rx, i.rx, dt).unwrap_or(0.0);
                    up += byte_rate(b.tx, i.tx, dt).unwrap_or(0.0);
                }
            }
            s.net_down_per_sec = Some(down);
            s.net_up_per_sec = Some(up);
        }
    }

    if let Some(temps) = &cur.temps {
        s.cpu_temp_c = temps
            .iter()
            .filter(|t| t.label.starts_with("CPU"))
            .map(|t| t.celsius)
            .reduce(f64::max);
        s.gpu_temp_c = mean(
            temps
                .iter()
                .filter(|t| t.label.starts_with("GPU"))
                .map(|t| t.celsius),
        );
        s.temps = Some(temps.clone());
    }
    s.fans = cur.fans.clone();
    s
}

#[cfg(test)]
mod tests;

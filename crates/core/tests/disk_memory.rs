//! Disk & Memory collector: real engine and SQLite, fake clock, fake counters.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use wrapped_core::collectors;
use wrapped_core::probe::{DiskMemoryReading, PressureLevel, SystemProbe};
use wrapped_core::time::MS_PER_MIN;
use wrapped_core::{Engine, ManualClock, Store, SystemEvent};

const CST: i32 = 480; // UTC+8
const NAME: &str = "disk_memory";
const BOOT: i64 = 1_790_000_000_000;
const MB: u64 = 1_000_000;

/// Counters the test moves by hand; counts how often they are read.
#[derive(Clone)]
struct Fake {
    r: Arc<Mutex<DiskMemoryReading>>,
    reads: Arc<AtomicUsize>,
}

impl Fake {
    fn new() -> Self {
        Self {
            r: Arc::new(Mutex::new(DiskMemoryReading {
                boot_time_ms: BOOT,
                disk_bytes_read: 10_000 * MB,
                disk_bytes_written: 5_000 * MB,
                swap_in_bytes: 0,
                swap_out_bytes: 0,
                pressure: PressureLevel::Normal,
                compressed_bytes: 1_000 * MB,
            })),
            reads: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn with(&self, f: impl FnOnce(&mut DiskMemoryReading)) {
        f(&mut self.r.lock().unwrap());
    }
    /// Add I/O: read, written, swap in, swap out (bytes).
    fn io(&self, read: u64, written: u64, swap_in: u64, swap_out: u64) {
        self.with(|r| {
            r.disk_bytes_read += read;
            r.disk_bytes_written += written;
            r.swap_in_bytes += swap_in;
            r.swap_out_bytes += swap_out;
        });
    }
    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl SystemProbe for Fake {
    fn disk_memory(&self) -> Option<DiskMemoryReading> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Some(*self.r.lock().unwrap())
    }
}

struct H {
    e: Engine,
    c: ManualClock,
    p: Fake,
}

impl H {
    fn at(h: u32, m: u32) -> Self {
        Self::at_enabled(h, m, true)
    }
    fn at_enabled(h: u32, m: u32, enabled: bool) -> Self {
        let c = ManualClock::at_local(2026, 11, 16, h, m, 0, CST);
        let store = Store::open_in_memory().unwrap();
        if enabled {
            store
                .set_setting("collector.disk_memory.enabled", "true")
                .unwrap();
        }
        let mut e = Engine::new(store, Arc::new(c.clone()), collectors::builtin()).unwrap();
        let p = Fake::new();
        e.set_probe(Arc::new(p.clone()));
        Self { e, c, p }
    }
    fn ev(&mut self, ev: SystemEvent) {
        self.e.handle(ev).unwrap();
    }
    fn tick(&mut self) {
        self.tick_input(1.0);
    }
    fn tick_input(&mut self, secs_since_input: f64) {
        self.ev(SystemEvent::IdleTick {
            secs_since_input,
            display_assertion_holders: vec![],
        });
    }
    /// `n` minutes, a tick at the end of each, `per_min` bytes written per minute.
    fn minutes(&mut self, n: i64, per_min_written: u64) {
        for _ in 0..n {
            self.p.io(per_min_written / 2, per_min_written, 0, 0);
            self.c.advance_secs(60);
            self.tick();
        }
    }
    /// (utc ms, metric, value) of every stored sample, after a flush.
    fn samples(&mut self) -> Vec<(i64, String, f64)> {
        self.e.flush().unwrap();
        let conn = self.e.store().conn();
        let mut st = conn
            .prepare(
                "SELECT time_ms, metric, value FROM samples WHERE collector = ?1
                 ORDER BY time_ms, metric",
            )
            .unwrap();
        st.query_map([NAME], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
    fn sample_values(&mut self, metric: &str) -> Vec<f64> {
        self.samples()
            .into_iter()
            .filter(|(_, m, _)| m == metric)
            .map(|(_, _, v)| v)
            .collect()
    }
    fn metric(&mut self, date: &str, metric: &str, key: &str) -> Option<f64> {
        self.e.flush().unwrap();
        self.e
            .store()
            .daily_metrics(date.parse().unwrap())
            .unwrap()
            .into_iter()
            .find(|r| r.collector == NAME && r.metric == metric && r.key == key)
            .map(|r| r.value)
    }
    fn metrics_of(&mut self, date: &str) -> Vec<(String, String, f64)> {
        self.e.flush().unwrap();
        self.e
            .store()
            .daily_metrics(date.parse().unwrap())
            .unwrap()
            .into_iter()
            .filter(|r| r.collector == NAME)
            .map(|r| (r.metric, r.key, r.value))
            .collect()
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn off_by_default_reads_nothing() {
    let mut h = H::at_enabled(9, 0, false);
    h.tick();
    h.minutes(20, MB);
    assert_eq!(h.p.reads(), 0);
    assert!(h.samples().is_empty());
}

#[test]
fn first_tick_baselines_then_one_sample_every_five_minutes() {
    let mut h = H::at(9, 0);
    h.tick(); // baseline at 09:00
    assert_eq!(h.p.reads(), 1);
    h.minutes(4, 2 * MB);
    assert_eq!(h.p.reads(), 1, "no read before 5 minutes");
    h.minutes(1, 2 * MB); // 09:05
    assert_eq!(h.p.reads(), 2);
    h.minutes(10, 2 * MB); // 09:10, 09:15
    assert_eq!(h.p.reads(), 4);
    assert_eq!(h.sample_values("bytes_written"), vec![10e6, 10e6, 10e6]);
    assert_eq!(h.sample_values("bytes_read"), vec![5e6, 5e6, 5e6]);
    assert_eq!(h.sample_values("secs"), vec![300.0, 300.0, 300.0]);
    assert_eq!(h.metric("2026-11-16", "bytes_written", ""), Some(30e6));
    assert_eq!(h.metric("2026-11-16", "bytes_read", ""), Some(15e6));
}

#[test]
fn ticks_a_little_early_still_sample_every_five_ticks() {
    let mut h = H::at(9, 0);
    h.tick();
    // Timer tolerance: ticks 58 s apart. The 5th tick (4 min 50 s) samples.
    for _ in 0..10 {
        h.p.io(0, MB, 0, 0);
        h.c.advance_secs(58);
        h.tick();
    }
    assert_eq!(h.sample_values("secs"), vec![290.0, 290.0]);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6, 5e6]);
}

#[test]
fn swap_and_memory_state_are_recorded() {
    let mut h = H::at(9, 0);
    h.tick();
    h.p.io(0, 0, 3 * MB, 7 * MB);
    h.p.with(|r| {
        r.pressure = PressureLevel::Warning;
        r.compressed_bytes = 2_000 * MB;
    });
    h.minutes(5, 0);
    h.p.with(|r| {
        r.pressure = PressureLevel::Normal;
        r.compressed_bytes = 1_000 * MB;
    });
    h.minutes(5, 0);
    assert_eq!(h.sample_values("swap_in_bytes"), vec![3e6, 0.0]);
    assert_eq!(h.sample_values("swap_out_bytes"), vec![7e6, 0.0]);
    assert_eq!(h.sample_values("pressure"), vec![1.0, 0.0]);
    let d = "2026-11-16";
    assert_eq!(h.metric(d, "swap_in_bytes", ""), Some(3e6));
    assert_eq!(h.metric(d, "swap_out_bytes", ""), Some(7e6));
    assert_eq!(h.metric(d, "compressed_peak_bytes", ""), Some(2_000e6));
    assert_eq!(h.metric(d, "compressed_avg_bytes", ""), Some(1_500e6));
    assert_eq!(
        h.metric(d, "pressure_secs_by_level", "warning"),
        Some(300.0)
    );
    assert_eq!(h.metric(d, "pressure_secs_by_level", "normal"), Some(300.0));
    assert_eq!(h.metric(d, "pressure_secs_by_level", "critical"), None);
}

#[test]
fn reboot_rebaselines_without_negative_or_huge_delta() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB); // one normal sample
    // "Reboot": new boot time, counters restart near zero.
    h.p.with(|r| {
        r.boot_time_ms = BOOT + 3_600_000;
        r.disk_bytes_read = 100;
        r.disk_bytes_written = 200;
    });
    h.minutes(5, MB); // re-baseline only
    h.minutes(5, MB); // normal again
    assert_eq!(h.sample_values("bytes_written"), vec![5e6, 5e6]);
    for (_, m, v) in h.samples() {
        assert!(v >= 0.0, "{m} negative: {v}");
    }
}

#[test]
fn reboot_with_higher_counters_is_not_a_huge_delta() {
    let mut h = H::at(9, 0);
    h.tick();
    // New boot whose counters are already above the old ones: a plain
    // "did it go down?" check would record 50 GB here.
    h.p.with(|r| {
        r.boot_time_ms = BOOT + 7_200_000;
        r.disk_bytes_written += 50_000 * MB;
    });
    h.minutes(5, MB);
    assert!(h.sample_values("bytes_written").is_empty());
    h.minutes(5, MB);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6]);
}

#[test]
fn counter_going_down_without_reboot_rebaselines() {
    let mut h = H::at(9, 0);
    h.tick();
    h.p.with(|r| r.swap_out_bytes = 0);
    h.p.io(0, 0, 0, 10 * MB);
    h.minutes(5, MB);
    h.p.with(|r| r.swap_out_bytes = 1); // went down
    h.minutes(5, MB);
    h.minutes(5, MB);
    assert_eq!(h.sample_values("swap_out_bytes"), vec![10e6, 0.0]);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6, 5e6]);
}

#[test]
fn impossible_rate_is_treated_as_reset() {
    let mut h = H::at(9, 0);
    h.tick();
    h.p.io(0, 100_000_000 * MB, 0, 0); // 100 TB in 5 minutes
    h.minutes(5, 0);
    assert!(h.sample_values("bytes_written").is_empty());
}

#[test]
fn sleep_gap_gives_one_delta_at_the_wake_tick_and_no_dark_wake_reads() {
    let mut h = H::at(22, 0);
    h.tick(); // baseline 22:00
    h.minutes(5, MB); // sample 22:05
    h.minutes(2, MB); // 22:07: 2 MB written since the sample
    h.ev(SystemEvent::WillSleep);
    let reads = h.p.reads();
    // Night with dark wakes: the minute timer fires, input is stale.
    for _ in 0..3 {
        h.c.advance_secs(2 * 3600);
        h.p.io(0, 4 * MB, 0, 0); // maintenance I/O
        h.tick_input(30_000.0);
    }
    assert_eq!(h.p.reads(), reads, "dark-wake ticks never read");
    h.c.advance_secs(20 * 60); // 04:27
    h.ev(SystemEvent::DidWake);
    h.c.advance_secs(30);
    h.p.io(0, MB, 0, 0);
    h.tick(); // 04:27:30, the wake tick
    let w = h.samples();
    let written: Vec<_> = w.iter().filter(|s| s.1 == "bytes_written").collect();
    assert_eq!(written.len(), 2, "{w:?}");
    // 2 MB before sleep + 12 MB in dark wakes + 1 MB after wake, one delta.
    assert_eq!(written[1].2, 15e6);
    let secs = h.sample_values("secs");
    // Awake time only: 2 min before sleep + 30 s after wake.
    assert_eq!(secs, vec![300.0, 150.0]);
    // The whole delta is on the wake day, in the hour of the wake tick.
    assert_eq!(h.metric("2026-11-17", "bytes_written", ""), Some(15e6));
    assert_eq!(
        h.metric("2026-11-17", "bytes_written_by_hour", "04"),
        Some(15e6)
    );
    assert_eq!(h.metric("2026-11-16", "bytes_written", ""), Some(5e6));
}

#[test]
fn missed_did_wake_input_tick_still_samples() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB);
    h.ev(SystemEvent::WillSleep);
    h.c.advance_secs(3600);
    h.p.io(0, 3 * MB, 0, 0);
    h.tick_input(1.0); // input after sleep: awake, no DidWake was seen
    assert!(!h.e.is_asleep());
    let w = h.sample_values("bytes_written");
    assert_eq!(w, vec![5e6, 3e6]);
}

#[test]
fn nothing_while_paused_and_resume_rebaselines() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB);
    h.e.set_pause(wrapped_core::Pause::Indefinite).unwrap();
    let reads = h.p.reads();
    h.minutes(30, 10 * MB);
    assert_eq!(h.p.reads(), reads);
    h.e.set_pause(wrapped_core::Pause::Off).unwrap();
    h.minutes(1, 10 * MB); // first tick after resume: new baseline
    h.minutes(5, MB);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6, 5e6]);
}

#[test]
fn pressure_seconds_add_up_to_sampled_time() {
    let mut h = H::at(9, 0);
    h.tick();
    let levels = [
        PressureLevel::Normal,
        PressureLevel::Warning,
        PressureLevel::Critical,
        PressureLevel::Warning,
        PressureLevel::Normal,
    ];
    for (i, l) in levels.iter().cycle().take(23).enumerate() {
        h.p.with(|r| r.pressure = *l);
        // Uneven tick spacing (59–63 s).
        for _ in 0..5 {
            h.p.io(0, MB, 0, 0);
            h.c.advance_secs(59 + (i as i64 % 5));
            h.tick();
        }
    }
    let d = "2026-11-16";
    let sampled: f64 = h.sample_values("secs").iter().sum();
    let by_level: f64 = ["normal", "warning", "critical"]
        .iter()
        .filter_map(|k| h.metric(d, "pressure_secs_by_level", k))
        .sum();
    assert!(sampled > 0.0);
    assert!(close(sampled, by_level), "{sampled} vs {by_level}");
    // Sampled time = wall time from the baseline (09:00) to the last sample.
    let last = h.samples().last().unwrap().0;
    let start = ManualClock::at_local(2026, 11, 16, 9, 0, 0, CST);
    use wrapped_core::Clock;
    assert!(close(sampled, (last - start.now().utc_ms) as f64 / 1000.0));
}

#[test]
fn midnight_split_of_a_sample_interval() {
    let mut h = H::at(23, 58);
    h.tick(); // baseline 23:58
    for _ in 0..5 {
        h.p.io(0, 100 * MB, 0, 0);
        h.p.with(|r| r.pressure = PressureLevel::Warning);
        h.c.advance_secs(60);
        h.tick();
    } // sample at 00:03 covering 23:58–00:03: 2/5 before midnight
    let (d1, d2) = ("2026-11-16", "2026-11-17");
    assert!(close(h.metric(d1, "bytes_written", "").unwrap(), 200e6));
    assert!(close(h.metric(d2, "bytes_written", "").unwrap(), 300e6));
    assert!(close(
        h.metric(d1, "bytes_written_by_hour", "23").unwrap(),
        200e6
    ));
    assert!(close(
        h.metric(d2, "bytes_written_by_hour", "00").unwrap(),
        300e6
    ));
    assert!(close(
        h.metric(d1, "pressure_secs_by_level", "warning").unwrap(),
        120.0
    ));
    assert!(close(
        h.metric(d2, "pressure_secs_by_level", "warning").unwrap(),
        180.0
    ));
    // Instant values belong to both days the window touches.
    assert_eq!(h.metric(d1, "compressed_peak_bytes", ""), Some(1_000e6));
    assert_eq!(h.metric(d2, "compressed_peak_bytes", ""), Some(1_000e6));
}

#[test]
fn hours_split_within_a_day() {
    let mut h = H::at(9, 57);
    h.tick();
    h.minutes(5, 60 * MB); // 09:57–10:02: 3/5 in hour 09
    let d = "2026-11-16";
    assert!(close(
        h.metric(d, "bytes_written_by_hour", "09").unwrap(),
        180e6
    ));
    assert!(close(
        h.metric(d, "bytes_written_by_hour", "10").unwrap(),
        120e6
    ));
    assert_eq!(h.metric(d, "bytes_written_by_hour", "11"), None);
}

#[test]
fn only_days_with_samples_get_metrics() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(10, MB);
    assert!(!h.metrics_of("2026-11-16").is_empty());
    assert!(h.metrics_of("2026-11-15").is_empty());
    assert!(h.metrics_of("2026-11-17").is_empty());
}

#[test]
fn delete_clears_samples_and_metrics_and_rebaselines() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(10, MB);
    assert!(!h.samples().is_empty());
    h.minutes(3, 50 * MB); // buffered I/O before the delete
    h.e.delete_collector_data(NAME).unwrap();
    assert!(h.samples().is_empty());
    assert!(h.metrics_of("2026-11-16").is_empty());
    h.minutes(5, MB);
    // Nothing from before the delete leaks into the next sample.
    assert_eq!(h.sample_values("bytes_written"), vec![5e6]);
    assert_eq!(h.metric("2026-11-16", "bytes_written", ""), Some(5e6));
}

#[test]
fn disable_with_delete_removes_everything() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(10, MB);
    h.e.disable_collector(NAME, false).unwrap();
    assert!(h.samples().is_empty());
    assert!(h.metrics_of("2026-11-16").is_empty());
    let reads = h.p.reads();
    h.minutes(10, MB);
    assert_eq!(h.p.reads(), reads);
}

#[test]
fn wall_clock_moving_back_rebaselines() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB);
    h.c.advance_ms(-30 * MS_PER_MIN);
    h.minutes(1, MB); // re-baseline
    h.minutes(5, MB);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6, 5e6]);
}

#[test]
fn utc_offset_change_uses_each_samples_own_offset() {
    let mut h = H::at(23, 50);
    h.tick();
    h.minutes(5, MB); // 23:55 at +08:00
    h.c.set_offset_min(-300); // fly to UTC-5: now 10:55 on the 16th
    h.minutes(5, MB);
    let d = "2026-11-16";
    assert!(close(h.metric(d, "bytes_written", "").unwrap(), 10e6));
    assert!(close(
        h.metric(d, "bytes_written_by_hour", "23").unwrap(),
        5e6
    ));
    assert!(close(
        h.metric(d, "bytes_written_by_hour", "10").unwrap(),
        5e6
    ));
    assert!(h.metrics_of("2026-11-17").is_empty());
}

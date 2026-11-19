//! Disk & Memory collector: real engine and SQLite, fake clock, fake counters.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use wrapped_core::collectors;
use wrapped_core::probe::{DiskMemoryReading, PressureLevel, SystemProbe};
use wrapped_core::time::MS_PER_MIN;
use wrapped_core::{Clock, Engine, ManualClock, Store, SystemEvent};

const CST: i32 = 480; // UTC+8
const NAME: &str = "disk_memory";
const BOOT: i64 = 1_790_000_000_000;
const MB: u64 = 1_000_000;

/// Counters the test moves by hand; counts how often they are read.
#[derive(Clone)]
struct Fake {
    r: Arc<Mutex<DiskMemoryReading>>,
    reads: Arc<AtomicUsize>,
    /// Reads fail (return None) while set.
    fail: Arc<std::sync::atomic::AtomicBool>,
}

impl Fake {
    fn new() -> Self {
        Self {
            r: Arc::new(Mutex::new(DiskMemoryReading {
                disk_only: false,
                boot_time_ms: BOOT,
                disk_bytes_read: 10_000 * MB,
                disk_bytes_written: 5_000 * MB,
                swap_in_bytes: 0,
                swap_out_bytes: 0,
                pressure: PressureLevel::Normal,
                compressed_bytes: 1_000 * MB,
            })),
            reads: Arc::new(AtomicUsize::new(0)),
            fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        if self.fail.load(Ordering::SeqCst) {
            return None;
        }
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
        let mut e =
            Engine::new(store, Arc::new(c.clone()), collectors::builtin_hermetic()).unwrap();
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
            display_assertions: vec![],
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
    /// Every stored reading flattened to (utc ms, value name, value), after a
    /// flush. Checks that each reading is exactly one `reading` row.
    fn samples(&mut self) -> Vec<(i64, String, f64)> {
        self.e.flush().unwrap();
        let conn = self.e.store().conn();
        let mut st = conn
            .prepare(
                "SELECT time_ms, metric, value, data FROM samples WHERE collector = ?1
                 ORDER BY time_ms, metric",
            )
            .unwrap();
        let rows: Vec<(i64, String, f64, String)> = st
            .query_map([NAME], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut out = Vec::new();
        for (t, metric, secs, data) in rows {
            assert_eq!(metric, "reading");
            let d: serde_json::Value = serde_json::from_str(&data).unwrap();
            let pressure = match d["p"].as_str().unwrap() {
                "normal" => 0.0,
                "warning" => 1.0,
                "critical" => 2.0,
                p => panic!("pressure {p}"),
            };
            out.push((t, "secs".to_string(), secs));
            for (name, key) in [
                ("bytes_read", "r"),
                ("bytes_written", "w"),
                ("swap_in_bytes", "si"),
                ("swap_out_bytes", "so"),
                ("compressed_bytes", "c"),
            ] {
                // Swap is left out when zero.
                let v = d.get(key).map(|v| v.as_f64().unwrap());
                assert!(v.is_some() || key.starts_with('s'), "{key} missing");
                out.push((t, name.to_string(), v.unwrap_or(0.0)));
            }
            out.push((t, "pressure".to_string(), pressure));
        }
        out
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
fn turned_off_while_paused_samples_again_after_resume() {
    let mut h = H::at(9, 0);
    h.tick();
    h.e.set_pause(wrapped_core::Pause::Indefinite).unwrap();
    h.e.disable_collector(NAME, true).unwrap();
    h.e.set_pause(wrapped_core::Pause::Off).unwrap();
    h.e.enable_collector(NAME).unwrap();
    h.minutes(1, MB); // baseline
    h.minutes(5, MB);
    assert_eq!(h.sample_values("bytes_written"), vec![5e6]);
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

#[test]
fn export_includes_reading_data() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB);
    h.samples();
    let mut buf = Vec::new();
    h.e.store().export_json(&mut buf, h.c.now()).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
    let text = v.to_string();
    assert!(text.contains("\"reading\""), "{text}");
    assert!(text.contains("\"w\":5000000"), "{text}");
}

#[test]
fn failed_reads_keep_the_baseline_and_the_window_is_capped_at_an_hour() {
    let mut h = H::at(22, 30);
    h.tick(); // baseline 22:30
    h.p.fail.store(true, Ordering::SeqCst);
    h.minutes(180, MB); // three hours of failed reads, past midnight
    assert!(h.samples().is_empty());
    h.p.fail.store(false, Ordering::SeqCst);
    h.minutes(1, MB); // 01:31: one reading for the whole gap
    assert_eq!(h.sample_values("secs"), vec![3600.0]);
    assert_eq!(h.sample_values("bytes_written"), vec![181e6]);
    // The capped window (00:31–01:31) lies on the 17th only.
    assert_eq!(h.metric("2026-11-17", "bytes_written", ""), Some(181e6));
    assert!(h.metrics_of("2026-11-16").is_empty());
}

#[test]
fn compressed_average_is_weighted_by_time() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB); // 300 s at 1,000 MB
    h.minutes(2, MB);
    h.ev(SystemEvent::WillSleep);
    h.c.advance_secs(3600);
    h.ev(SystemEvent::DidWake);
    h.c.advance_secs(30);
    h.p.with(|r| r.compressed_bytes = 4_000 * MB);
    h.tick(); // 150 s (120 before sleep + 30 after) at 4,000 MB
    assert_eq!(h.sample_values("secs"), vec![300.0, 150.0]);
    // (1000 * 300 + 4000 * 150) / 450 = 2000, not the plain mean 2500.
    let avg = h.metric("2026-11-16", "compressed_avg_bytes", "").unwrap();
    assert!(close(avg, 2_000e6), "{avg}");
    assert_eq!(
        h.metric("2026-11-16", "compressed_peak_bytes", ""),
        Some(4_000e6)
    );
}

#[test]
fn delete_during_a_dark_wake_does_not_count_the_sleep_as_awake() {
    let mut h = H::at(9, 0);
    h.tick();
    h.minutes(5, MB);
    h.ev(SystemEvent::WillSleep);
    h.c.advance_secs(600);
    h.tick_input(9_000.0); // dark wake
    h.e.delete_collector_data(NAME).unwrap();
    h.c.advance_secs(3000);
    h.ev(SystemEvent::DidWake);
    h.c.advance_secs(300);
    h.tick();
    // Awake since the wake only: 300 s, not the 50 minutes of sleep.
    assert_eq!(h.sample_values("secs"), vec![300.0]);
}

#[test]
fn disk_only_readings_leave_unavailable_memory_metrics_absent() {
    let ticks = [wrapped_core::collectors::disk_memory::Tick {
        disk_only: true,
        time: wrapped_core::Timestamp::new(300_000, 0),
        secs: 300.,
        bytes_read: 120.,
        bytes_written: 60.,
        swap_in: 0.,
        swap_out: 0.,
        pressure: PressureLevel::Normal,
        compressed: 0.,
    }];
    let metrics = wrapped_core::collectors::disk_memory::rollup_ticks(&ticks, 0);
    assert_eq!(metrics.len(), 3);
    assert!(
        metrics
            .iter()
            .any(|m| m.metric == "bytes_written" && m.value == 60.)
    );
}

//! Shared helpers for the stats tests and the stats benchmark: write sessions,
//! items and samples through the real store and the real collectors' rollups,
//! and generate synthetic data for any number of days.

#![allow(dead_code)]

use chrono::NaiveDate;
use wrapped_core::collector::{
    Collector, DayInput, ItemRecord, MetricValue, SampleRecord, SessionRecord,
};
use wrapped_core::store::Batch;
use wrapped_core::time::{MS_PER_HOUR, MS_PER_MIN, MS_PER_SEC, local_midnight_ms};
use wrapped_core::{Settings, Store, Timestamp, collectors};

pub const ALL: &[&str] = &["app_usage", "active_idle", "git", "photos", "disk_memory"];

/// A store with exactly these collectors turned on.
pub fn store_with(enabled: &[&str]) -> Store {
    let s = Store::open_in_memory().unwrap();
    for c in collectors::builtin() {
        let name = c.info().name;
        s.set_setting(
            &format!("collector.{name}.enabled"),
            if enabled.contains(&name) {
                "true"
            } else {
                "false"
            },
        )
        .unwrap();
    }
    s
}

pub fn settings(s: &Store) -> Settings {
    s.load_settings().unwrap().0
}

/// The moment `h:m` local on `date` in `offset_min`.
pub fn at(date: NaiveDate, h: i64, m: i64, offset_min: i32) -> Timestamp {
    let local = local_midnight_ms(date) + h * MS_PER_HOUR + m * MS_PER_MIN;
    Timestamp::new(local - i64::from(offset_min) * MS_PER_MIN, offset_min)
}

pub fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// Records waiting to be written in one flush.
#[derive(Default)]
pub struct Pending {
    pub sessions: Vec<SessionRecord>,
    pub items: Vec<ItemRecord>,
    pub samples: Vec<SampleRecord>,
}

impl Pending {
    pub fn session(
        &mut self,
        collector: &'static str,
        key: &str,
        start: Timestamp,
        end: Timestamp,
    ) {
        self.sessions.push(SessionRecord {
            collector,
            key: key.to_string(),
            start,
            end,
        });
    }
    pub fn commit(&mut self, id: &str, time: Timestamp, repo: &str) {
        self.items.push(ItemRecord {
            collector: "git",
            id: id.to_string(),
            time,
            offset_known: true,
            attrs: serde_json::json!({ "repo": repo }).to_string(),
        });
    }
    pub fn photo(
        &mut self,
        id: &str,
        time: Timestamp,
        offset_known: bool,
        camera: &str,
        lens: &str,
    ) {
        self.items.push(ItemRecord {
            collector: "photos",
            id: id.to_string(),
            time,
            offset_known,
            attrs: serde_json::json!({
                "make": "Sony", "model": camera, "lens": lens, "focal_mm": 35.0,
            })
            .to_string(),
        });
    }
    /// One Disk & Memory reading covering the 5 minutes before `time`.
    pub fn reading(&mut self, time: Timestamp, written: f64, compressed: f64) {
        self.samples.push(SampleRecord {
            collector: "disk_memory",
            metric: "reading".into(),
            time,
            value: 300.0,
            data: Some(
                serde_json::json!({ "r": written / 2.0, "w": written, "c": compressed, "p": "normal" })
                    .to_string(),
            ),
        });
    }

    /// Write everything in one flush, computing daily metrics with the real
    /// collectors' rollups.
    pub fn flush(self, store: &mut Store, now: Timestamp) {
        let all = collectors::builtin();
        let rollup = |name: &str, day: &DayInput<'_>| -> Option<Vec<MetricValue>> {
            all.iter()
                .find(|c| c.info().name == name)
                .map(|c| c.rollup(day))
        };
        let batch = Batch {
            closed: &self.sessions,
            open: &[],
            samples: &self.samples,
            items: &self.items,
            scans: &[],
        };
        store.flush(&batch, now, &rollup).unwrap();
    }
}

/// Small deterministic generator (xorshift).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// How much synthetic data per day.
#[derive(Clone, Copy)]
pub struct Scale {
    /// App sessions per active day (spread over 09:00–23:30).
    pub app_sessions: u32,
    pub commits_max: u64,
    pub photos_max: u64,
    /// Disk & Memory readings per day (288 = every 5 min all day).
    pub readings: u32,
}

pub const REALISTIC: Scale = Scale {
    app_sessions: 250,
    commits_max: 12,
    photos_max: 60,
    readings: 192,
};

/// `days` days of data for every collector from `from`, in `offset_min`,
/// one flush per day. Returns the number of sessions written.
pub fn synth(
    store: &mut Store,
    from: NaiveDate,
    days: u32,
    offset_min: i32,
    scale: Scale,
) -> usize {
    let mut rng = Rng::new(42);
    let mut n_sessions = 0;
    let mut item = 0u64;
    for i in 0..days {
        let date = from + chrono::Duration::days(i64::from(i));
        let mut p = Pending::default();
        // Roughly one day in seven without screen time.
        let active = rng.below(7) != 0;
        if active {
            let start = at(date, 9, 0, offset_min);
            let span = 14 * MS_PER_HOUR + 30 * MS_PER_MIN;
            let step = span / i64::from(scale.app_sessions.max(1));
            let mut t = start.utc_ms;
            for k in 0..scale.app_sessions {
                let len = step - (rng.below(30) as i64) * MS_PER_SEC;
                let app = format!("com.example.app{}", rng.below(30));
                let s = Timestamp::new(t, offset_min);
                let e = Timestamp::new(t + len.max(MS_PER_SEC), offset_min);
                if k % 40 == 39 {
                    p.session("active_idle", "idle", s, e);
                } else {
                    p.session("app_usage", &app, s, e);
                }
                t += step;
            }
            p.session(
                "active_idle",
                "sleep",
                at(date, 23, 45, offset_min),
                at(date + chrono::Duration::days(1), 8, 30, offset_min),
            );
            n_sessions += scale.app_sessions as usize + 1;
        }
        for _ in 0..rng.below(scale.commits_max + 1) {
            item += 1;
            let t = at(
                date,
                10 + rng.below(12) as i64,
                rng.below(60) as i64,
                offset_min,
            );
            p.commit(&format!("c{item}"), t, &format!("repo{}", rng.below(6)));
        }
        if rng.below(4) == 0 {
            for _ in 0..rng.below(scale.photos_max + 1) {
                item += 1;
                let t = at(
                    date,
                    8 + rng.below(12) as i64,
                    rng.below(60) as i64,
                    offset_min,
                );
                let cam = ["ILCE-7M4", "ILCE-6700", "iPhone"][rng.below(3) as usize];
                p.photo(&format!("p{item}"), t, true, cam, "FE 35mm F1.8");
            }
        }
        let first = 24 * 60 - i64::from(scale.readings) * 5;
        for k in 0..i64::from(scale.readings) {
            let m = first + (k + 1) * 5;
            let t = at(date, m / 60, m % 60, offset_min);
            p.reading(
                t,
                1e6 + rng.below(50) as f64 * 1e5,
                1e9 + rng.below(9) as f64 * 1e8,
            );
        }
        p.flush(
            store,
            at(date + chrono::Duration::days(1), 9, 0, offset_min),
        );
    }
    n_sessions
}

pub fn collectors_all() -> Vec<Box<dyn Collector>> {
    collectors::builtin()
}

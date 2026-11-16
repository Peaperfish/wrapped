//! The collector plugin interface (DESIGN.md §6) and the [`Sink`] collectors
//! emit through. Collectors never touch the disk: everything they emit lands in
//! an in-memory [`Buffer`] that the shared batched writer flushes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use chrono::NaiveDate;

use crate::event::SystemEvent;
use crate::settings::Settings;
use crate::time::{Clock, MS_PER_HOUR, MS_PER_SEC, Timestamp};

/// Live collectors react to system events; scanned collectors read new data on
/// demand (git, photos, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Live,
    Scanned,
}

/// Static description used for onboarding and the Settings toggle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorInfo {
    /// Stable id, used in the database and setting keys. Never rename.
    pub name: &'static str,
    pub title: &'static str,
    /// What it records, in plain words.
    pub description: &'static str,
    /// Permission needed, or "None".
    pub permissions: &'static str,
    /// Battery / disk cost, in plain words.
    pub cost: &'static str,
    pub default_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    Count,
}

/// One metric a collector contributes to `daily_metrics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricDef {
    pub id: &'static str,
    pub title: &'static str,
    pub unit: Unit,
    /// What `daily_metrics.key` means for this metric (`None` = always "").
    pub key: Option<&'static str>,
}

/// One computed `daily_metrics` row (date and collector are implied).
#[derive(Debug, Clone, PartialEq)]
pub struct MetricValue {
    pub metric: &'static str,
    pub key: String,
    pub value: f64,
}

impl MetricValue {
    pub fn new(metric: &'static str, key: impl Into<String>, value: f64) -> Self {
        Self {
            metric,
            key: key.into(),
            value,
        }
    }
}

/// The part of one stored session that falls inside one local day.
///
/// The day is decided using the session's *start* offset (sessions are split
/// by the collector within a minute of an offset change, see [`Sink::set_session`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub key: String,
    pub start: Timestamp,
    pub end: Timestamp,
    /// True when this slice begins where the stored session begins (i.e. it is
    /// not the after-midnight continuation of a session from the previous day).
    pub starts_session: bool,
}

impl Slice {
    pub fn duration_ms(&self) -> i64 {
        (self.end.utc_ms - self.start.utc_ms).max(0)
    }
    pub fn secs(&self) -> f64 {
        self.duration_ms() as f64 / MS_PER_SEC as f64
    }
    /// Seconds per local hour of day (0..=23) covered by this slice.
    pub fn hour_secs(&self) -> Vec<(u32, f64)> {
        let mut out = Vec::new();
        let off = self.start.local_ms() - self.start.utc_ms;
        let (mut t, end) = (self.start.local_ms(), self.end.utc_ms + off);
        while t < end {
            let hour_start = t - t.rem_euclid(MS_PER_HOUR);
            let next = (hour_start + MS_PER_HOUR).min(end);
            let hour = (t.rem_euclid(24 * MS_PER_HOUR) / MS_PER_HOUR) as u32;
            out.push((hour, (next - t) as f64 / MS_PER_SEC as f64));
            t = next;
        }
        out
    }
}

/// Everything a collector needs to compute one local day of metrics.
#[derive(Debug, Clone)]
pub struct DayInput<'a> {
    pub date: NaiveDate,
    /// This collector's session slices for the day, sorted by start.
    pub slices: &'a [Slice],
}

/// The plugin trait. Adding a collector = one new file in `collectors/` plus
/// one line in the `register!` list there.
///
/// Differences from the sketch in DESIGN.md §6:
/// * live collectors receive events through [`Collector::on_event`] rather than
///   subscribing to the OS themselves, so they stay pure and testable;
/// * [`Collector::rollup`] lets the framework recompute daily metrics without
///   knowing anything about the collector.
pub trait Collector: Send {
    fn info(&self) -> CollectorInfo;
    fn kind(&self) -> Kind;
    /// Begin collecting. The collector keeps `out` and emits through it.
    fn start(&mut self, out: Sink);
    /// Live collectors: one system event. `now` is read from [`Sink::now`].
    fn on_event(&mut self, _ev: &SystemEvent) {}
    /// Scanned collectors: read new data since the last scan.
    fn scan(&mut self, _since: Option<Timestamp>, _out: Sink) {}
    /// Stop collecting; must close anything open at the current time.
    fn stop(&mut self);
    fn metrics(&self) -> Vec<MetricDef>;
    /// Compute this collector's `daily_metrics` rows for one local day.
    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue>;
}

/// A finished (or currently open, when flushed) session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub collector: &'static str,
    pub key: String,
    pub start: Timestamp,
    pub end: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenSession {
    pub key: String,
    pub start: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampleRecord {
    pub collector: &'static str,
    pub metric: String,
    pub time: Timestamp,
    pub value: f64,
}

/// In-memory records waiting for the next flush.
#[derive(Debug, Default)]
pub struct Buffer {
    /// Closed sessions in the order they closed. Zero-length ones are kept so
    /// the writer can delete a row it previously wrote while it was open.
    pub closed: Vec<SessionRecord>,
    /// At most one open session per collector.
    pub open: BTreeMap<&'static str, OpenSession>,
    pub samples: Vec<SampleRecord>,
}

impl Buffer {
    pub fn is_empty(&self) -> bool {
        self.closed.is_empty() && self.open.is_empty() && self.samples.is_empty()
    }
}

/// State shared by the engine and every sink.
pub(crate) struct Shared {
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) buffer: Mutex<Buffer>,
    pub(crate) settings: RwLock<Arc<Settings>>,
}

impl Shared {
    pub(crate) fn new(clock: Arc<dyn Clock>, settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            clock,
            buffer: Mutex::new(Buffer::default()),
            settings: RwLock::new(Arc::new(settings)),
        })
    }
}

/// A collector's handle to the framework: the clock, the current settings and
/// the in-memory buffer. Cheap to clone.
#[derive(Clone)]
pub struct Sink {
    collector: &'static str,
    shared: Arc<Shared>,
}

impl Sink {
    pub(crate) fn new(collector: &'static str, shared: Arc<Shared>) -> Self {
        Self { collector, shared }
    }

    /// A sink with its own private buffer, for testing a collector in isolation.
    pub fn standalone(collector: &'static str, clock: Arc<dyn Clock>, settings: Settings) -> Self {
        Self::new(collector, Shared::new(clock, settings))
    }

    pub fn now(&self) -> Timestamp {
        self.shared.clock.now()
    }

    pub fn settings(&self) -> Arc<Settings> {
        self.shared.settings.read().expect("settings lock").clone()
    }

    /// Replace the settings seen by this sink's buffer (tests / standalone use).
    pub fn set_settings(&self, s: Settings) {
        *self.shared.settings.write().expect("settings lock") = Arc::new(s);
    }

    /// Declare which session (if any) this collector should have open from
    /// `at` onwards. Idempotent: calling it with the current key changes nothing.
    ///
    /// * A different key (or `None`) closes the open session at `at` (never
    ///   before its start) and opens the new one there.
    /// * The same key but a different UTC offset than the session started with
    ///   (travel, DST) splits the session at `at`, so every stored session has
    ///   a single offset. Collectors call this on every event, including the
    ///   minute tick, so a split lands within about a minute of the change.
    pub fn set_session(&self, key: Option<&str>, at: Timestamp) {
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        if let (Some(open), Some(k)) = (b.open.get(self.collector), key)
            && open.key == k
            && open.start.offset_min == at.offset_min
        {
            return;
        }
        let mut start = at;
        if let Some(open) = b.open.remove(self.collector) {
            let end = if at < open.start { open.start } else { at };
            start = Timestamp::new(end.utc_ms, at.offset_min);
            b.closed.push(SessionRecord {
                collector: self.collector,
                key: open.key,
                start: open.start,
                end,
            });
        }
        if let Some(k) = key {
            b.open.insert(
                self.collector,
                OpenSession {
                    key: k.to_string(),
                    start,
                },
            );
        }
    }

    /// Key of this collector's open session, if any.
    pub fn open_key(&self) -> Option<String> {
        let b = self.shared.buffer.lock().expect("buffer lock");
        b.open.get(self.collector).map(|o| o.key.clone())
    }

    /// Record one periodic sample (for counter-style collectors).
    pub fn sample(&self, metric: &str, time: Timestamp, value: f64) {
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        b.samples.push(SampleRecord {
            collector: self.collector,
            metric: metric.to_string(),
            time,
            value,
        });
    }

    /// Take everything buffered so far (tests; the engine drains via `Shared`).
    pub fn take_buffer(&self) -> Buffer {
        std::mem::take(&mut *self.shared.buffer.lock().expect("buffer lock"))
    }

    /// Snapshot of the buffer without draining it.
    pub fn peek_closed(&self) -> Vec<SessionRecord> {
        self.shared
            .buffer
            .lock()
            .expect("buffer lock")
            .closed
            .clone()
    }

    pub fn peek_open(&self) -> Option<OpenSession> {
        self.shared
            .buffer
            .lock()
            .expect("buffer lock")
            .open
            .get(self.collector)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{MS_PER_MIN, ManualClock};

    fn sink() -> (Sink, ManualClock) {
        let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, 0);
        (
            Sink::standalone("t", Arc::new(c.clone()), Settings::default()),
            c,
        )
    }

    #[test]
    fn set_session_open_switch_close() {
        let (s, c) = sink();
        let t0 = c.now();
        s.set_session(Some("a"), t0);
        s.set_session(Some("a"), t0.plus_ms(5_000)); // no-op
        s.set_session(Some("b"), t0.plus_ms(10_000));
        s.set_session(None, t0.plus_ms(15_000));
        let b = s.take_buffer();
        assert!(b.open.is_empty());
        assert_eq!(b.closed.len(), 2);
        assert_eq!(b.closed[0].key, "a");
        assert_eq!(b.closed[0].end.utc_ms - b.closed[0].start.utc_ms, 10_000);
        assert_eq!(b.closed[1].start, b.closed[0].end);
    }

    #[test]
    fn close_never_before_start() {
        let (s, c) = sink();
        let t0 = c.now();
        s.set_session(Some("a"), t0);
        s.set_session(None, t0.plus_ms(-60_000));
        let b = s.take_buffer();
        assert_eq!(b.closed[0].start, b.closed[0].end);
    }

    #[test]
    fn offset_change_splits() {
        let (s, c) = sink();
        let t0 = c.now();
        s.set_session(Some("a"), t0);
        let later = Timestamp::new(t0.utc_ms + 5 * MS_PER_MIN, 60);
        s.set_session(Some("a"), later);
        let b = s.take_buffer();
        assert_eq!(b.closed.len(), 1);
        assert_eq!(b.closed[0].start.offset_min, 0);
        assert_eq!(b.open["t"].start.offset_min, 60);
        assert_eq!(b.open["t"].start.utc_ms, later.utc_ms);
    }

    #[test]
    fn hour_secs_split_by_local_hour() {
        let c = ManualClock::at_local(2026, 11, 16, 9, 50, 0, 330); // +05:30
        let start = c.now();
        let sl = Slice {
            key: "a".into(),
            start,
            end: start.plus_ms(25 * MS_PER_MIN),
            starts_session: true,
        };
        assert_eq!(sl.hour_secs(), vec![(9, 600.0), (10, 900.0)]);
    }
}

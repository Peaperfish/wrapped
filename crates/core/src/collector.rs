//! The collector plugin interface (DESIGN.md §6) and the [`Sink`] collectors
//! emit through. Collectors never touch the disk: everything they emit lands in
//! an in-memory [`Buffer`] that the shared batched writer flushes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

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

/// One stored item (commit, photo) whose local date is the day being rolled up.
#[derive(Debug, Clone, PartialEq)]
pub struct DayItem {
    pub id: String,
    /// For an item without a known offset this is the naive local time with
    /// offset 0, so `local_ms()` / `local_date()` are still right.
    pub time: Timestamp,
    pub attrs: serde_json::Value,
}

impl DayItem {
    /// Local hour of day, 0..=23.
    pub fn local_hour(&self) -> u32 {
        (self.time.local_ms().rem_euclid(24 * MS_PER_HOUR) / MS_PER_HOUR) as u32
    }
    /// A string attribute, if present and non-empty.
    pub fn attr_str(&self, k: &str) -> Option<&str> {
        self.attrs.get(k)?.as_str().filter(|s| !s.is_empty())
    }
    pub fn attr_f64(&self, k: &str) -> Option<f64> {
        self.attrs.get(k)?.as_f64()
    }
}

/// Everything a collector needs to compute one local day of metrics.
#[derive(Debug, Clone)]
pub struct DayInput<'a> {
    pub date: NaiveDate,
    /// This collector's session slices for the day, sorted by start.
    pub slices: &'a [Slice],
    /// This collector's items whose local date is `date`, sorted by time.
    pub items: &'a [DayItem],
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
    /// Scanned collectors: read new data since the last scan. Runs on a
    /// background thread (on a fresh instance from [`Collector::scanner`]).
    /// `since` is the start of the last completed scan (`None` = never).
    /// Must check [`Sink::cancelled`] often and, when it is set, save progress
    /// with [`Sink::save_scan_state`] and return.
    fn scan(&mut self, _since: Option<Timestamp>, _out: Sink) {}
    /// Scanned collectors: a fresh instance to run [`Collector::scan`] on the
    /// background thread, so this one keeps serving `rollup` meanwhile.
    fn scanner(&self) -> Option<Box<dyn Collector>> {
        None
    }
    /// Settings to store explicitly when the collector is turned on, given the
    /// current settings (e.g. git's author emails). Empty by default.
    fn enable_defaults(&self, _settings: &Settings) -> Vec<(String, String)> {
        Vec::new()
    }
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

/// One item emitted by a scanned collector.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRecord {
    pub collector: &'static str,
    /// Dedupe key: the same id is stored once, however often it is emitted.
    pub id: String,
    pub time: Timestamp,
    /// False when the source has no UTC offset; `time` is then the naive local
    /// time with offset 0 (stored with `time_off` NULL).
    pub offset_known: bool,
    /// JSON object text.
    pub attrs: String,
}

/// Per-file incremental cache entry (see `scan_files`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub path_hash: u64,
    pub size: u64,
    pub mtime_ms: i64,
}

/// A scanned collector's pending bookkeeping, written in the same transaction
/// as the items emitted before it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScanUpdate {
    /// New `scan_state.state` JSON (replaces the stored one).
    pub state: Option<String>,
    pub files_put: Vec<FileStamp>,
    pub files_del: Vec<u64>,
    /// Set when a scan completed: `scan_state.last_scanned_ms` = scan start.
    pub done_at: Option<Timestamp>,
}

impl ScanUpdate {
    pub fn is_empty(&self) -> bool {
        self.state.is_none()
            && self.files_put.is_empty()
            && self.files_del.is_empty()
            && self.done_at.is_none()
    }
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
    /// Items from scanned collectors, in emission order.
    pub items: Vec<ItemRecord>,
    pub scans: BTreeMap<&'static str, ScanUpdate>,
}

impl Buffer {
    pub fn is_empty(&self) -> bool {
        self.closed.is_empty()
            && self.open.is_empty()
            && self.samples.is_empty()
            && self.items.is_empty()
            && self.scans.values().all(ScanUpdate::is_empty)
    }
    /// Scan output waiting to be written (drives early flushes / backpressure).
    pub fn scan_pending(&self) -> usize {
        self.items.len()
            + self
                .scans
                .values()
                .map(|u| u.files_put.len() + u.files_del.len())
                .sum::<usize>()
    }
}

/// Pending scan output at which the writer is woken for an early flush.
pub const SCAN_FLUSH_AT: usize = 1_000;
/// Pending scan output at which a scanning thread waits for the writer, so a
/// scan's memory stays bounded however much it finds.
pub const SCAN_BLOCK_AT: usize = 5_000;

/// Optional per-platform tweaks for scans (set by the platform layer).
#[derive(Clone, Copy, Default)]
pub struct ScanHooks {
    /// Runs first on every scan thread (macOS: background QoS).
    pub thread_setup: Option<fn()>,
    /// Applied to every child process a scan starts (macOS: background priority).
    pub command_setup: Option<fn(&mut std::process::Command)>,
}

/// One running scan's shared context: cancellation, the previous incremental
/// state, the per-file cache and the counters shown in the scan report.
pub struct ScanCtx {
    stop: AtomicBool,
    discard: AtomicBool,
    prev_state: Option<String>,
    /// Sorted by `path_hash`.
    files: Vec<FileStamp>,
    visited: Mutex<Vec<bool>>,
    counters: Mutex<BTreeMap<&'static str, u64>>,
    /// Test knob: report cancelled after this many `cancelled()` checks.
    cancel_after: Option<AtomicU64>,
    hooks: ScanHooks,
}

impl ScanCtx {
    pub fn new(prev_state: Option<String>, mut files: Vec<FileStamp>, hooks: ScanHooks) -> Self {
        files.sort_by_key(|f| f.path_hash);
        let n = files.len();
        Self {
            stop: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            prev_state,
            files,
            visited: Mutex::new(vec![false; n]),
            counters: Mutex::new(BTreeMap::new()),
            cancel_after: None,
            hooks,
        }
    }
    /// Test knob: behave as if cancelled after `n` checks of `cancelled()`.
    pub fn with_cancel_after(mut self, n: Option<u64>) -> Self {
        self.cancel_after = n.map(AtomicU64::new);
        self
    }
    /// Ask the scan to stop, keeping (saving) what it has done so far.
    pub fn cancel(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    /// Stop and drop everything it emits from now on (data being deleted).
    pub fn abort(&self) {
        self.discard.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
    fn discarding(&self) -> bool {
        self.discard.load(Ordering::SeqCst)
    }
    pub fn counters(&self) -> BTreeMap<&'static str, u64> {
        self.counters.lock().expect("counters lock").clone()
    }
}

/// Wakes the writer (main thread) for an early flush.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// State shared by the engine and every sink.
pub(crate) struct Shared {
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) buffer: Mutex<Buffer>,
    pub(crate) settings: RwLock<Arc<Settings>>,
    /// Signalled after every drain of the buffer (wakes blocked scan threads).
    pub(crate) drained: Condvar,
    /// Signalled when a scan thread wants a flush or has finished.
    pub(crate) activity: Condvar,
    pub(crate) waker: RwLock<Option<Waker>>,
    /// A wake-up is already on its way to the writer.
    pub(crate) wake_sent: AtomicBool,
    /// An engine drains this buffer (standalone sinks have no writer).
    pub(crate) has_writer: AtomicBool,
}

impl Shared {
    pub(crate) fn new(clock: Arc<dyn Clock>, settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            clock,
            buffer: Mutex::new(Buffer::default()),
            settings: RwLock::new(Arc::new(settings)),
            drained: Condvar::new(),
            activity: Condvar::new(),
            waker: RwLock::new(None),
            wake_sent: AtomicBool::new(false),
            has_writer: AtomicBool::new(false),
        })
    }

    /// Ask the writer for a flush (once until the next drain).
    pub(crate) fn wake_writer(&self) {
        self.activity.notify_all();
        if !self.wake_sent.swap(true, Ordering::SeqCst)
            && let Some(w) = self.waker.read().expect("waker lock").clone()
        {
            w();
        }
    }
}

/// A collector's handle to the framework: the clock, the current settings and
/// the in-memory buffer. Cheap to clone.
#[derive(Clone)]
pub struct Sink {
    collector: &'static str,
    shared: Arc<Shared>,
    scan: Option<Arc<ScanCtx>>,
}

impl Sink {
    pub(crate) fn new(collector: &'static str, shared: Arc<Shared>) -> Self {
        Self {
            collector,
            shared,
            scan: None,
        }
    }

    pub(crate) fn for_scan(
        collector: &'static str,
        shared: Arc<Shared>,
        ctx: Arc<ScanCtx>,
    ) -> Self {
        Self {
            collector,
            shared,
            scan: Some(ctx),
        }
    }

    /// A standalone sink carrying a scan context (testing a scanned collector
    /// without an engine). Nothing drains it, so backpressure never blocks.
    pub fn standalone_scan(
        collector: &'static str,
        clock: Arc<dyn Clock>,
        settings: Settings,
        ctx: Arc<ScanCtx>,
    ) -> Self {
        Self::for_scan(collector, Shared::new(clock, settings), ctx)
    }

    // ---------- scanned collectors ----------

    /// The scan should stop now (quit, sleep, pause, turned off). Collectors
    /// check this between files / repos.
    pub fn cancelled(&self) -> bool {
        let Some(ctx) = &self.scan else { return false };
        if let Some(n) = &ctx.cancel_after
            && n.try_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
                .is_err()
        {
            ctx.cancel();
        }
        ctx.is_cancelled()
    }

    fn discarding(&self) -> bool {
        self.scan.as_ref().is_some_and(|c| c.discarding())
    }

    /// The `scan_state.state` JSON saved by the previous scan, if any.
    pub fn scan_state(&self) -> Option<&str> {
        self.scan.as_ref()?.prev_state.as_deref()
    }

    /// Save incremental state (written with the items emitted before it).
    pub fn save_scan_state(&self, json: String) {
        if self.discarding() {
            return;
        }
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        b.scans.entry(self.collector).or_default().state = Some(json);
    }

    /// Platform tweak for child processes started by a scan.
    pub fn configure_command(&self, cmd: &mut std::process::Command) {
        if let Some(f) = self.scan.as_ref().and_then(|c| c.hooks.command_setup) {
            f(cmd);
        }
    }

    /// Add to a counter shown in the scan report.
    pub fn count(&self, key: &'static str, n: u64) {
        if let Some(ctx) = &self.scan {
            *ctx.counters
                .lock()
                .expect("counters lock")
                .entry(key)
                .or_default() += n;
        }
    }

    /// Per-file cache: true if this file was seen by an earlier scan with the
    /// same size and mtime (so it need not be read). Marks it as still present.
    pub fn file_unchanged(&self, stamp: FileStamp) -> bool {
        let Some(ctx) = &self.scan else { return false };
        match ctx
            .files
            .binary_search_by_key(&stamp.path_hash, |f| f.path_hash)
        {
            Ok(i) => {
                ctx.visited.lock().expect("visited lock")[i] = true;
                ctx.files[i] == stamp
            }
            Err(_) => false,
        }
    }

    /// Per-file cache: this file has been processed (emit its items first).
    pub fn file_done(&self, stamp: FileStamp) {
        if self.discarding() {
            return;
        }
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        b.scans
            .entry(self.collector)
            .or_default()
            .files_put
            .push(stamp);
        self.after_push(b);
    }

    /// After a complete walk: forget cache entries for files no longer there.
    pub fn prune_unseen_files(&self) {
        let Some(ctx) = &self.scan else { return };
        if self.discarding() {
            return;
        }
        let gone: Vec<u64> = {
            let v = ctx.visited.lock().expect("visited lock");
            ctx.files
                .iter()
                .zip(v.iter())
                .filter(|(_, seen)| !**seen)
                .map(|(f, _)| f.path_hash)
                .collect()
        };
        if gone.is_empty() {
            return;
        }
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        b.scans
            .entry(self.collector)
            .or_default()
            .files_del
            .extend(gone);
        self.after_push(b);
    }

    /// Emit one item (commit, photo). Blocks while too much scan output is
    /// waiting for the writer.
    pub fn item(&self, id: String, time: Timestamp, offset_known: bool, attrs: String) {
        if self.discarding() {
            return;
        }
        let mut b = self.shared.buffer.lock().expect("buffer lock");
        b.items.push(ItemRecord {
            collector: self.collector,
            id,
            time,
            offset_known,
            attrs,
        });
        self.after_push(b);
    }

    fn after_push(&self, mut b: std::sync::MutexGuard<'_, Buffer>) {
        let pending = b.scan_pending();
        if pending < SCAN_FLUSH_AT || self.scan.is_none() {
            return;
        }
        self.shared.wake_writer();
        // A cancelled scan is about to stop; let it save its progress.
        while b.scan_pending() >= SCAN_BLOCK_AT
            && !self.scan.as_ref().is_some_and(|c| c.is_cancelled())
        {
            // Only engine-run scans have a writer; never block a standalone sink.
            if !self.shared.has_writer.load(Ordering::SeqCst) {
                return;
            }
            let (g, _) = self
                .shared
                .drained
                .wait_timeout(b, Duration::from_millis(250))
                .expect("buffer lock");
            b = g;
            self.shared.wake_writer();
        }
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

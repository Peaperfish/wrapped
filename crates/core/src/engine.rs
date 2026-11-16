//! The engine: owns the collectors, the shared in-memory buffer and the
//! store, routes system events, and runs the flush policy.
//!
//! Flush policy: buffered records are written in one transaction when ~10 min
//! of wall-clock time have passed since the last flush (checked on every
//! event, so in practice on the minute tick), and immediately on `WillSleep`,
//! `WillPowerOff` and [`Engine::shutdown`] (quit / SIGTERM / SIGINT).
//! Between `WillSleep` and `DidWake` the periodic flush is skipped: macOS runs
//! the minute timer during dark wakes (maintenance wakes with no wake
//! notification), but everything was flushed at `WillSleep` and the open sleep
//! period is written at wake. A tick showing input after `WillSleep` also ends
//! this state, in case the wake notification was missed.
//! The engine is single-threaded: the platform layer calls it from one thread.
//!
//! Scanned collectors (git, photos) run on a background thread each, at most
//! one scan per collector at a time. The engine starts a scan on the first
//! minute tick at least 24 h after the start of the last completed one (or
//! never-scanned), never while paused, plus on demand ([`Engine::start_scan`]).
//! Scans emit into the same buffer; the engine flushes early when a scan has
//! [`SCAN_FLUSH_AT`] records pending (woken through the platform's waker) and
//! when a scan ends. Sleep, power-off, quit, pause and turning the collector
//! off cancel a scan; it saves its progress and the next scan resumes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::collector::{
    Buffer, Collector, Kind, SCAN_FLUSH_AT, ScanCtx, ScanHooks, ScanUpdate, Shared, Sink, Waker,
};
use crate::event::SystemEvent;
use crate::probe::SystemProbe;
use crate::settings::{KEY_PAUSE, Pause, Settings};
use crate::store::{Batch, Error, FlushReport, Result, Store};
use crate::time::{Clock, MS_PER_DAY, MS_PER_MIN, Timestamp};

pub const FLUSH_INTERVAL_MS: i64 = 10 * MS_PER_MIN;
/// Minimum time between the starts of two completed scans of one collector.
pub const SCAN_INTERVAL_MS: i64 = MS_PER_DAY;
/// How long cancelling waits for a scan thread to notice before moving on
/// (the thread then finishes on its own and is reaped later).
pub const SCAN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// What a scan did (shown by `wrapped-collector scan`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    pub collector: &'static str,
    pub started: Timestamp,
    pub finished: Timestamp,
    /// False if it was cancelled (or panicked) before the end.
    pub completed: bool,
    pub counters: BTreeMap<&'static str, u64>,
}

struct RunningScan {
    name: &'static str,
    ctx: Arc<ScanCtx>,
    handle: JoinHandle<()>,
    started: Timestamp,
}

type Logger = Box<dyn Fn(&str) + Send>;

struct Slot {
    collector: Box<dyn Collector>,
    name: &'static str,
    default_on: bool,
    running: bool,
}

pub struct Engine {
    store: Store,
    shared: Arc<Shared>,
    slots: Vec<Slot>,
    settings: Settings,
    paused: bool,
    last_flush_ms: i64,
    flush_interval_ms: i64,
    /// UTC ms of the last `WillSleep`, until `DidWake` (or input after it).
    asleep_since_ms: Option<i64>,
    last_front: Option<SystemEvent>,
    own_app_id: Option<String>,
    recovered: usize,
    log: Option<Logger>,
    scans: Vec<RunningScan>,
    /// Per scanned collector: start of the last completed scan, or of the
    /// last failed one (so a failing scan is retried a day later, not every
    /// minute). Loaded from `scan_state` on first use.
    last_scan_ms: BTreeMap<&'static str, Option<i64>>,
    auto_scans: bool,
    scan_hooks: ScanHooks,
    scan_cancel_after: Option<u64>,
    reports: Vec<ScanReport>,
    /// Settings fingerprints already checked against the store.
    fingerprints: BTreeMap<&'static str, String>,
}

impl Engine {
    /// Recover any session left open by a crash, load settings, start the
    /// enabled collectors and restore the pause state.
    pub fn new(
        mut store: Store,
        clock: Arc<dyn Clock>,
        collectors: Vec<Box<dyn Collector>>,
    ) -> Result<Self> {
        let recovered = store.close_open_sessions()?;
        let (settings, _skipped) = store.load_settings()?;
        let now = clock.now();
        let shared = Shared::new(clock, settings.clone());
        let slots = collectors
            .into_iter()
            .map(|c| {
                let info = c.info();
                Slot {
                    collector: c,
                    name: info.name,
                    default_on: info.default_on,
                    running: false,
                }
            })
            .collect();
        let mut e = Self {
            store,
            shared,
            slots,
            settings: Settings::default(),
            paused: false,
            last_flush_ms: now.utc_ms,
            flush_interval_ms: FLUSH_INTERVAL_MS,
            asleep_since_ms: None,
            last_front: None,
            own_app_id: None,
            recovered,
            log: None,
            scans: Vec::new(),
            last_scan_ms: BTreeMap::new(),
            auto_scans: true,
            scan_hooks: ScanHooks::default(),
            scan_cancel_after: None,
            reports: Vec::new(),
            fingerprints: BTreeMap::new(),
        };
        e.shared.has_writer.store(true, Ordering::SeqCst);
        e.apply_settings(settings)?;
        Ok(e)
    }

    /// Debug logger (off unless set).
    pub fn set_logger(&mut self, log: impl Fn(&str) + Send + 'static) {
        self.log = Some(Box::new(log));
    }

    /// The collector's own app identifier (supplied by the platform layer).
    /// FrontApp events for it are ignored: no session change, no switch, no
    /// activity evidence, so the previous app's session simply continues.
    pub fn set_own_app_id(&mut self, id: impl Into<String>) {
        self.own_app_id = Some(id.into());
    }

    pub fn set_flush_interval_ms(&mut self, ms: i64) {
        self.flush_interval_ms = ms;
    }

    /// Called from a scan thread when it wants the writer to flush; the
    /// platform layer should arrange for [`Engine::poll_scans`] to run soon on
    /// the engine's thread. Without one, scans are flushed on the next event.
    pub fn set_waker(&mut self, w: Waker) {
        *self.shared.waker.write().expect("waker lock") = Some(w);
    }

    /// System counters for the Disk & Memory collector (platform layer).
    pub fn set_probe(&mut self, probe: Arc<dyn SystemProbe>) {
        *self.shared.probe.write().expect("probe lock") = probe;
    }

    pub fn set_scan_hooks(&mut self, hooks: ScanHooks) {
        self.scan_hooks = hooks;
    }

    /// Daily scheduling on (default) or off (the `scan` CLI runs one scan only).
    pub fn set_auto_scans(&mut self, on: bool) {
        self.auto_scans = on;
    }

    /// Test knob: every scan started from now on behaves as cancelled after
    /// `n` checks of [`Sink::cancelled`].
    pub fn set_scan_cancel_after(&mut self, n: Option<u64>) {
        self.scan_cancel_after = n;
    }

    fn log(&self, msg: impl FnOnce() -> String) {
        if let Some(l) = &self.log {
            l(&msg());
        }
    }

    pub fn now(&self) -> Timestamp {
        self.shared.clock.now()
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Sessions closed by crash recovery at startup.
    pub fn recovered_sessions(&self) -> usize {
        self.recovered
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Between `WillSleep` and `DidWake`: a tick now is a dark-wake tick.
    pub fn is_asleep(&self) -> bool {
        self.asleep_since_ms.is_some()
    }

    pub fn running_collectors(&self) -> Vec<&'static str> {
        self.slots
            .iter()
            .filter(|s| s.running)
            .map(|s| s.name)
            .collect()
    }

    /// Handle one system event. Returns the flush report if it caused a flush.
    pub fn handle(&mut self, ev: SystemEvent) -> Result<Option<FlushReport>> {
        let tick = matches!(ev, SystemEvent::IdleTick { .. });
        if tick {
            // Pick up changes made by another process (CLI) and expire timed pauses.
            match self.store.load_settings() {
                Ok((s, _)) => self.apply_settings(s)?,
                Err(e) => self.log(|| format!("settings reload failed: {e}")),
            }
        }
        if matches!(ev, SystemEvent::WillSleep | SystemEvent::WillPowerOff) {
            self.cancel_scans(None, false);
        }
        // Update the asleep state first, so collectors handling this event
        // (disk_memory skips dark-wake ticks) see the engine's current view.
        let now = self.now();
        match &ev {
            SystemEvent::WillSleep => self.asleep_since_ms = Some(now.utc_ms),
            SystemEvent::DidWake => self.asleep_since_ms = None,
            SystemEvent::IdleTick {
                secs_since_input, ..
            } => {
                let secs = if secs_since_input.is_finite() {
                    secs_since_input.max(0.0)
                } else {
                    0.0
                };
                let last_input = now.utc_ms - (secs * 1000.0) as i64;
                if self.asleep_since_ms.is_some_and(|since| last_input > since) {
                    self.asleep_since_ms = None; // input after sleep: awake
                }
            }
            _ => {}
        }
        self.shared
            .asleep
            .store(self.asleep_since_ms.is_some(), Ordering::SeqCst);
        let own = matches!(&ev, SystemEvent::FrontApp { bundle_id, .. }
            if self.own_app_id.as_deref() == Some(bundle_id.as_str()));
        if own {
            self.log(|| "event FrontApp <own app, ignored>".to_string());
        } else {
            if matches!(ev, SystemEvent::FrontApp { .. }) {
                self.last_front = Some(ev.clone());
            }
            self.dispatch(&ev);
        }

        if now.utc_ms < self.last_flush_ms {
            self.last_flush_ms = now.utc_ms; // wall clock moved backwards
        }
        // While asleep (dark-wake ticks) no periodic flush and no new scans:
        // everything was flushed and scans were cancelled at WillSleep.
        let asleep = self.asleep_since_ms.is_some();
        let mut due = !asleep && now.utc_ms - self.last_flush_ms >= self.flush_interval_ms;
        if tick {
            due |= self.reap_scans();
            if self.auto_scans && !asleep {
                self.schedule_scans()?;
            }
        }
        due |= self.scan_pending() >= SCAN_FLUSH_AT;
        if due || matches!(ev, SystemEvent::WillSleep | SystemEvent::WillPowerOff) {
            return self.flush().map(Some);
        }
        Ok(None)
    }

    // ---------- scans ----------

    fn scan_pending(&self) -> usize {
        self.shared
            .buffer
            .lock()
            .expect("buffer lock")
            .scan_pending()
    }

    /// Reap finished scans and flush if a scan ended or has a lot pending.
    /// The platform's waker should lead here.
    pub fn poll_scans(&mut self) -> Result<Option<FlushReport>> {
        let reaped = self.reap_scans();
        if reaped || self.scan_pending() >= SCAN_FLUSH_AT {
            return self.flush().map(Some);
        }
        Ok(None)
    }

    /// Names of collectors with a scan thread still running.
    pub fn scanning(&self) -> Vec<&'static str> {
        self.scans.iter().map(|s| s.name).collect()
    }

    /// Reports of scans that ended since the last call.
    pub fn take_scan_reports(&mut self) -> Vec<ScanReport> {
        std::mem::take(&mut self.reports)
    }

    /// Start of the last completed scan (UTC ms), from memory or `scan_state`.
    pub fn last_scan_ms(&mut self, name: &'static str) -> Result<Option<i64>> {
        if let Some(v) = self.last_scan_ms.get(name) {
            return Ok(*v);
        }
        let v = self.store.scan_info(name)?.last_scanned_ms;
        self.last_scan_ms.insert(name, v);
        Ok(v)
    }

    /// Start a scan of `name` now (on demand). Errors if the collector is
    /// unknown, not scanned, turned off, or tracking is paused. Returns false
    /// if one is already running.
    pub fn start_scan(&mut self, name: &str) -> Result<bool> {
        let i = self.slot_index(name)?;
        let slot = &self.slots[i];
        if slot.collector.kind() != Kind::Scanned {
            return Err(Error::Scan(format!("`{name}` is not a scanned collector")));
        }
        if !slot.running {
            return Err(Error::Scan(format!(
                "`{name}` is turned off (settings set collector.{name}.enabled true)"
            )));
        }
        if self.paused {
            return Err(Error::Scan("tracking is paused".into()));
        }
        if self.scans.iter().any(|s| s.name == name) {
            return Ok(false);
        }
        self.spawn_scan(i)?;
        Ok(true)
    }

    fn spawn_scan(&mut self, i: usize) -> Result<()> {
        let name = self.slots[i].name;
        let Some(mut scanner) = self.slots[i].collector.scanner() else {
            return Err(Error::Scan(format!("`{name}` has no scanner")));
        };
        let info = self.store.scan_info(name)?;
        let files = self.store.scan_files(name)?;
        let ctx = Arc::new(
            ScanCtx::new(info.state, files, self.scan_hooks)
                .with_cancel_after(self.scan_cancel_after),
        );
        let since = info.last_scanned_ms.map(|ms| Timestamp::new(ms, 0));
        let sink = Sink::for_scan(name, self.shared.clone(), ctx.clone());
        let setup = self.scan_hooks.thread_setup;
        let shared = self.shared.clone();
        let started = self.now();
        let handle = std::thread::Builder::new()
            .name(format!("scan-{name}"))
            .spawn(move || {
                if let Some(f) = setup {
                    f();
                }
                scanner.start(sink.clone());
                scanner.scan(since, sink);
                // Wake the writer to reap and flush.
                shared.wake_writer();
            })?;
        self.log(|| format!("scan {name} started"));
        self.scans.push(RunningScan {
            name,
            ctx,
            handle,
            started,
        });
        Ok(())
    }

    /// Start every enabled scanned collector that is due (or was requested by
    /// another process), unless paused.
    fn schedule_scans(&mut self) -> Result<()> {
        let requested = match self.store.take_scan_requests() {
            Ok(r) => r,
            Err(e) => {
                self.log(|| format!("reading scan requests failed: {e}"));
                Vec::new()
            }
        };
        if self.paused {
            return Ok(());
        }
        let now = self.now().utc_ms;
        for i in 0..self.slots.len() {
            let slot = &self.slots[i];
            let name = slot.name;
            if slot.collector.kind() != Kind::Scanned
                || !slot.running
                || self.scans.iter().any(|s| s.name == name)
            {
                continue;
            }
            let due = match self.last_scan_ms(name)? {
                None => true,
                Some(last) => now - last >= SCAN_INTERVAL_MS || now < last,
            };
            if (due || requested.iter().any(|r| r == name))
                && let Err(e) = self.spawn_scan(i)
            {
                self.log(|| format!("scan {name} could not start: {e}"));
            }
        }
        Ok(())
    }

    /// Join finished scan threads, record completed ones. True if any ended.
    fn reap_scans(&mut self) -> bool {
        let mut ended = false;
        let mut i = 0;
        while i < self.scans.len() {
            if !self.scans[i].handle.is_finished() {
                i += 1;
                continue;
            }
            let s = self.scans.swap_remove(i);
            self.finish_scan(s);
            ended = true;
        }
        ended
    }

    fn finish_scan(&mut self, s: RunningScan) {
        let ok = s.handle.join().is_ok();
        let cancelled = s.ctx.is_cancelled();
        let completed = ok && !cancelled;
        if completed {
            let mut b = self.shared.buffer.lock().expect("buffer lock");
            b.scans.entry(s.name).or_default().done_at = Some(s.started);
        }
        if completed || !ok {
            // A panicking scan is retried a day later, not on every tick.
            self.last_scan_ms.insert(s.name, Some(s.started.utc_ms));
        }
        let report = ScanReport {
            collector: s.name,
            started: s.started,
            finished: self.now(),
            completed,
            counters: s.ctx.counters(),
        };
        self.log(|| {
            format!(
                "scan {} {}: {:?}",
                s.name,
                if completed {
                    "completed"
                } else if ok {
                    "cancelled (progress saved)"
                } else {
                    "FAILED (panicked)"
                },
                report.counters
            )
        });
        self.reports.push(report);
    }

    /// Cancel running scans (all, or one collector's). `discard` drops what
    /// they emit from now on (their data is being deleted). Waits up to
    /// [`SCAN_JOIN_TIMEOUT`] for each thread; one still running afterwards is
    /// reaped later.
    pub fn cancel_scans(&mut self, only: Option<&str>, discard: bool) {
        let mut waiting = false;
        for s in self
            .scans
            .iter()
            .filter(|s| only.is_none_or(|n| n == s.name))
        {
            if discard {
                s.ctx.abort();
            } else {
                s.ctx.cancel();
            }
            waiting = true;
        }
        if !waiting {
            return;
        }
        // Unblock any thread waiting for the writer.
        self.shared.drained.notify_all();
        let deadline = Instant::now() + SCAN_JOIN_TIMEOUT;
        while Instant::now() < deadline
            && self
                .scans
                .iter()
                .any(|s| only.is_none_or(|n| n == s.name) && !s.handle.is_finished())
        {
            let b = self.shared.buffer.lock().expect("buffer lock");
            let _ = self
                .shared
                .activity
                .wait_timeout(b, Duration::from_millis(10))
                .expect("buffer lock");
        }
        self.reap_scans();
    }

    /// Block until every running scan ends, flushing as output piles up
    /// (used by the `scan` CLI). Returns the reports.
    pub fn wait_scans(&mut self) -> Result<Vec<ScanReport>> {
        while !self.scans.is_empty() {
            {
                let b = self.shared.buffer.lock().expect("buffer lock");
                if b.scan_pending() < SCAN_FLUSH_AT {
                    let _ = self
                        .shared
                        .activity
                        .wait_timeout(b, Duration::from_millis(200))
                        .expect("buffer lock");
                }
            }
            self.poll_scans()?;
        }
        self.flush()?;
        Ok(self.take_scan_reports())
    }

    fn dispatch(&mut self, ev: &SystemEvent) {
        self.log(|| match ev {
            // Excluded apps leave no trace, not even in debug output.
            SystemEvent::FrontApp { bundle_id, name }
                if self.settings.is_excluded(
                    &crate::presence::FrontApp {
                        bundle_id: bundle_id.clone(),
                        name: name.clone(),
                    }
                    .key(),
                ) =>
            {
                "event FrontApp <excluded app>".to_string()
            }
            _ => format!("event {ev:?}"),
        });
        for s in self.slots.iter_mut().filter(|s| s.running) {
            s.collector.on_event(ev);
        }
    }

    /// Write everything buffered now, in one transaction.
    pub fn flush(&mut self) -> Result<FlushReport> {
        let now = self.now();
        let (closed, samples, open, items, scans) = {
            let mut b = self.shared.buffer.lock().expect("buffer lock");
            let closed = std::mem::take(&mut b.closed);
            let samples = std::mem::take(&mut b.samples);
            let items = std::mem::take(&mut b.items);
            let scans: Vec<(&'static str, ScanUpdate)> =
                std::mem::take(&mut b.scans).into_iter().collect();
            let open: Vec<_> = b.open.iter().map(|(c, o)| (*c, o.clone())).collect();
            self.shared.wake_sent.store(false, Ordering::SeqCst);
            (closed, samples, open, items, scans)
        };
        // Blocked scan threads may continue as soon as the buffer is drained.
        self.shared.drained.notify_all();
        let slots = &self.slots;
        let rollup = |name: &str, day: &crate::collector::DayInput<'_>| {
            slots
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.collector.rollup(day))
        };
        let batch = Batch {
            closed: &closed,
            open: &open,
            samples: &samples,
            items: &items,
            scans: &scans,
        };
        match self.store.flush(&batch, now, &rollup) {
            Ok(r) => {
                // Success is logged by the caller (the platform layer adds
                // byte counts); failures are logged here.
                self.last_flush_ms = now.utc_ms;
                Ok(r)
            }
            Err(e) => {
                // Keep the records for the next attempt; nothing is lost.
                let mut b = self.shared.buffer.lock().expect("buffer lock");
                let mut kept = Buffer {
                    closed,
                    samples,
                    items,
                    ..Default::default()
                };
                kept.closed.append(&mut b.closed);
                kept.samples.append(&mut b.samples);
                kept.items.append(&mut b.items);
                b.closed = kept.closed;
                b.samples = kept.samples;
                b.items = kept.items;
                // Older scan bookkeeping goes first; newer state wins.
                let newer = std::mem::take(&mut b.scans);
                for (name, mut old) in scans {
                    if let Some(n) = newer.get(name) {
                        old.files_put.extend(n.files_put.iter().copied());
                        old.files_del.extend(n.files_del.iter().copied());
                        if n.state.is_some() {
                            old.state = n.state.clone();
                        }
                        if n.done_at.is_some() {
                            old.done_at = n.done_at;
                        }
                    }
                    b.scans.insert(name, old);
                }
                for (name, n) in newer {
                    b.scans.entry(name).or_insert(n);
                }
                self.log(|| format!("flush failed: {e}"));
                Err(e)
            }
        }
    }

    /// Stop every collector (closing open sessions now) and flush. Call on
    /// quit, SIGTERM and SIGINT.
    pub fn shutdown(&mut self) -> Result<FlushReport> {
        self.cancel_scans(None, false);
        for s in self.slots.iter_mut().filter(|s| s.running) {
            s.collector.stop();
            s.running = false;
        }
        self.flush()
    }

    /// Pause / resume tracking (tray). Persisted immediately.
    pub fn set_pause(&mut self, pause: Pause) -> Result<()> {
        self.store
            .set_setting(KEY_PAUSE, &Settings::pause_json(pause))?;
        let mut s = self.settings.clone();
        s.pause = pause;
        self.apply_settings(s)
    }

    /// Turn a collector off: stop it (closing its open session), flush, then
    /// persist the setting and optionally delete its data.
    pub fn disable_collector(&mut self, name: &str, keep_data: bool) -> Result<()> {
        self.slot_index(name)?;
        self.cancel_scans(Some(name), !keep_data);
        if let Some(slot) = self.slots.iter_mut().find(|s| s.name == name && s.running) {
            slot.collector.stop();
            slot.running = false;
        }
        self.flush()?;
        self.store.disable_collector(name, keep_data)?;
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)
    }

    pub fn enable_collector(&mut self, name: &str) -> Result<()> {
        let i = self.slot_index(name)?;
        for (k, v) in self.slots[i].collector.enable_defaults(&self.settings) {
            self.store.set_setting(&k, &v)?;
        }
        self.store
            .set_setting(&crate::settings::collector_enabled_key(name), "true")?;
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)
    }

    /// Delete everything a collector recorded (it keeps running if enabled).
    pub fn delete_collector_data(&mut self, name: &str) -> Result<()> {
        let i = self.slot_index(name)?;
        self.cancel_scans(Some(name), true);
        let name = self.slots[i].name;
        self.last_scan_ms.remove(name);
        let now = self.now();
        // Drop its buffered records so they are not written back afterwards,
        // and restart any open session now so no pre-delete time survives.
        {
            let mut b = self.shared.buffer.lock().expect("buffer lock");
            b.closed.retain(|r| r.collector != name);
            b.samples.retain(|r| r.collector != name);
            b.items.retain(|r| r.collector != name);
            b.scans.remove(name);
            if let Some(o) = b.open.get_mut(name) {
                o.start = now;
            }
        }
        if self.slots[i].running {
            self.slots[i].collector.on_data_deleted();
        }
        self.store.delete_collector_data(name)
    }

    fn slot_index(&self, name: &str) -> Result<usize> {
        self.slots
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| {
                Error::Settings(crate::settings::SettingsError::UnknownKey(format!(
                    "collector.{name}"
                )))
            })
    }

    /// Reconcile running collectors and pause state with `new` settings.
    fn apply_settings(&mut self, mut new: Settings) -> Result<()> {
        let now = self.now();
        // A timed pause that has run out: clear it in the database too.
        if let Pause::Until(_) = new.pause
            && !new.pause.is_active(now)
        {
            new.pause = Pause::Off;
            self.store
                .set_setting(KEY_PAUSE, &Settings::pause_json(Pause::Off))?;
        }
        *self.shared.settings.write().expect("settings lock") = Arc::new(new.clone());

        let want_paused = new.pause.is_active(now);
        for i in 0..self.slots.len() {
            let want = new.collector_enabled(self.slots[i].name, self.slots[i].default_on);
            let slot = &mut self.slots[i];
            if want && !slot.running {
                slot.collector
                    .start(Sink::new(slot.name, self.shared.clone()));
                slot.running = true;
                if slot.collector.kind() == Kind::Live {
                    // Bring a newly started collector up to date.
                    if self.paused {
                        slot.collector.on_event(&SystemEvent::Paused);
                    }
                    if let Some(f) = &self.last_front {
                        slot.collector.on_event(f);
                    }
                }
            } else if !want && slot.running {
                slot.collector.stop();
                slot.running = false;
                let name = slot.name;
                self.cancel_scans(Some(name), false);
            }
        }
        if want_paused != self.paused {
            self.paused = want_paused;
            if want_paused {
                self.cancel_scans(None, false);
            }
            self.dispatch(if want_paused {
                &SystemEvent::Paused
            } else {
                &SystemEvent::Resumed
            });
        }
        self.settings = new;
        self.sync_scan_fingerprints()
    }

    /// A running scanned collector whose counting rule changed (git author
    /// emails added or removed) starts over: its data, incremental state and
    /// metrics are deleted, so the next tick (or `scan`) reads everything again
    /// and counts exactly what the new rule says.
    fn sync_scan_fingerprints(&mut self) -> Result<()> {
        for i in 0..self.slots.len() {
            let slot = &self.slots[i];
            if !slot.running || slot.collector.kind() != Kind::Scanned {
                continue;
            }
            let Some(fp) = slot.collector.scan_fingerprint(&self.settings) else {
                continue;
            };
            let name = slot.name;
            if self.fingerprints.get(name) == Some(&fp) {
                continue;
            }
            if self.store.scan_fingerprint(name)?.as_deref() != Some(fp.as_str()) {
                self.log(|| {
                    format!("{name}: counting settings changed; data reset for a full rescan")
                });
                self.delete_collector_data(name)?;
                self.store.set_scan_fingerprint(name, &fp)?;
            }
            self.fingerprints.insert(name, fp);
        }
        Ok(())
    }
}

//! The engine: owns the collectors, the shared in-memory buffer and the
//! store, routes system events, and runs the flush policy.
//!
//! Flush policy: buffered records are written in one transaction when ~10 min
//! of wall-clock time have passed since the last flush (checked on every
//! event, so in practice on the minute tick), and immediately on `WillSleep`,
//! `WillPowerOff` and [`Engine::shutdown`] (quit / SIGTERM / SIGINT).
//! The engine is single-threaded: the platform layer calls it from one thread.

use std::sync::Arc;

use crate::collector::{Buffer, Collector, Kind, Shared, Sink};
use crate::event::SystemEvent;
use crate::settings::{KEY_PAUSE, Pause, Settings};
use crate::store::{Error, FlushReport, Result, Store};
use crate::time::{Clock, MS_PER_MIN, Timestamp};

pub const FLUSH_INTERVAL_MS: i64 = 10 * MS_PER_MIN;

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
    last_front: Option<SystemEvent>,
    recovered: usize,
    log: Option<Logger>,
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
            last_front: None,
            recovered,
            log: None,
        };
        e.apply_settings(settings)?;
        Ok(e)
    }

    /// Debug logger (off unless set).
    pub fn set_logger(&mut self, log: impl Fn(&str) + Send + 'static) {
        self.log = Some(Box::new(log));
    }

    pub fn set_flush_interval_ms(&mut self, ms: i64) {
        self.flush_interval_ms = ms;
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

    pub fn running_collectors(&self) -> Vec<&'static str> {
        self.slots
            .iter()
            .filter(|s| s.running)
            .map(|s| s.name)
            .collect()
    }

    /// Handle one system event. Returns the flush report if it caused a flush.
    pub fn handle(&mut self, ev: SystemEvent) -> Result<Option<FlushReport>> {
        if matches!(ev, SystemEvent::IdleTick { .. }) {
            // Pick up changes made by another process (CLI) and expire timed pauses.
            match self.store.load_settings() {
                Ok((s, _)) => self.apply_settings(s)?,
                Err(e) => self.log(|| format!("settings reload failed: {e}")),
            }
        }
        if matches!(ev, SystemEvent::FrontApp { .. }) {
            self.last_front = Some(ev.clone());
        }
        self.dispatch(&ev);

        let now = self.now();
        if now.utc_ms < self.last_flush_ms {
            self.last_flush_ms = now.utc_ms; // wall clock moved backwards
        }
        let due = now.utc_ms - self.last_flush_ms >= self.flush_interval_ms;
        if due || matches!(ev, SystemEvent::WillSleep | SystemEvent::WillPowerOff) {
            return self.flush().map(Some);
        }
        Ok(None)
    }

    fn dispatch(&mut self, ev: &SystemEvent) {
        self.log(|| format!("event {ev:?}"));
        for s in self.slots.iter_mut().filter(|s| s.running) {
            s.collector.on_event(ev);
        }
    }

    /// Write everything buffered now, in one transaction.
    pub fn flush(&mut self) -> Result<FlushReport> {
        let now = self.now();
        let (closed, samples, open) = {
            let mut b = self.shared.buffer.lock().expect("buffer lock");
            let closed = std::mem::take(&mut b.closed);
            let samples = std::mem::take(&mut b.samples);
            let open: Vec<_> = b.open.iter().map(|(c, o)| (*c, o.clone())).collect();
            (closed, samples, open)
        };
        let slots = &self.slots;
        let rollup = |name: &str, day: &crate::collector::DayInput<'_>| {
            slots
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.collector.rollup(day))
        };
        match self.store.flush(&closed, &open, &samples, now, &rollup) {
            Ok(r) => {
                self.last_flush_ms = now.utc_ms;
                self.log(|| format!("flush {r:?}"));
                Ok(r)
            }
            Err(e) => {
                // Keep the records for the next attempt; nothing is lost.
                let mut b = self.shared.buffer.lock().expect("buffer lock");
                let mut kept = Buffer {
                    closed,
                    samples,
                    ..Default::default()
                };
                kept.closed.append(&mut b.closed);
                kept.samples.append(&mut b.samples);
                b.closed = kept.closed;
                b.samples = kept.samples;
                self.log(|| format!("flush failed: {e}"));
                Err(e)
            }
        }
    }

    /// Stop every collector (closing open sessions now) and flush. Call on
    /// quit, SIGTERM and SIGINT.
    pub fn shutdown(&mut self) -> Result<FlushReport> {
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
        self.slot_index(name)?;
        self.store
            .set_setting(&crate::settings::collector_enabled_key(name), "true")?;
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)
    }

    /// Delete everything a collector recorded (it keeps running if enabled).
    pub fn delete_collector_data(&mut self, name: &str) -> Result<()> {
        self.slot_index(name)?;
        let now = self.now();
        // Drop its buffered records so they are not written back afterwards,
        // and restart any open session now so no pre-delete time survives.
        {
            let mut b = self.shared.buffer.lock().expect("buffer lock");
            b.closed.retain(|r| r.collector != name);
            b.samples.retain(|r| r.collector != name);
            if let Some(o) = b.open.get_mut(name) {
                o.start = now;
            }
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
            }
        }
        if want_paused != self.paused {
            self.paused = want_paused;
            self.dispatch(if want_paused {
                &SystemEvent::Paused
            } else {
                &SystemEvent::Resumed
            });
        }
        self.settings = new;
        Ok(())
    }
}

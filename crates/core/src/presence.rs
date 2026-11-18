//! Is the user at the computer? A pure state machine over [`SystemEvent`]s.
//!
//! Both v1 collectors embed one of these: app usage records time while the
//! user is present, active/idle records the periods they are away.
//!
//! Rules (DESIGN.md §13):
//! * Idle after `idle_threshold_secs` without input; front apps on the
//!   "active without input" list use `active_without_input.timeout_secs`.
//! * Any process holding a keep-display-awake assertion keeps the user active.
//! * Sleep, display sleep, screen lock, and the lock screen / screensaver
//!   being in front all mean away. Power-off halts tracking.
//!
//! Where idle starts. "Last evidence of use" is the latest of: last input, an
//! app switch, wake / unlock / resume, or a tick where a display assertion was
//! held.
//! * Normal apps: idle is only checked on the ~60 s tick, so when it is
//!   detected the transition is backdated to the last evidence of use.
//! * Apps on the "active without input" list: not backdated. The session ends
//!   at last evidence of use + that list's timeout (the moment the longer
//!   timeout ran out), which is neither the tick time nor the last input.
//!   Time spent reading with no input is therefore kept, up to the timeout.
//!
//! Returning from idle is only noticed at the next tick (or the next app
//! switch / unlock / wake event), and is placed at the last input seen at that
//! tick, so the first session after an idle period can start up to ~60 s late.

use crate::event::{AwayReason, SystemEvent, away_app_reason};
use crate::settings::Settings;
use crate::time::{MS_PER_MIN, MS_PER_SEC, Timestamp};

/// If two idle ticks are further apart than this, the process did not run in
/// between (the machine slept without telling us). Treated as sleep.
pub const TICK_GAP_MS: i64 = 5 * MS_PER_MIN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontApp {
    pub bundle_id: String,
    pub name: String,
}

impl FrontApp {
    /// Stable session key: the bundle id, or `exe:<name>` for apps without one.
    pub fn key(&self) -> String {
        if self.bundle_id.is_empty() {
            format!("exe:{}", self.name)
        } else {
            self.bundle_id.clone()
        }
    }
}

/// The presence state at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub front: Option<FrontApp>,
    /// `Some` while the user is away.
    pub away: Option<AwayReason>,
    /// The machine is shutting down: record nothing.
    pub halted: bool,
}

impl View {
    pub fn present(&self) -> bool {
        self.away.is_none() && !self.halted
    }
}

/// A state the collectors should reflect from `at` onwards. One event can
/// produce several (e.g. a missed sleep detected late).
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub at: Timestamp,
    pub view: View,
}

#[derive(Debug, Clone, Default)]
pub struct Presence {
    front: Option<FrontApp>,
    idle: bool,
    /// Away states that input can disprove carry the UTC ms they began, so
    /// only input *after* that moment clears them.
    asleep: Option<i64>,
    screens_off: Option<i64>,
    locked: bool,
    powering_off: Option<i64>,
    /// Latest UTC ms with evidence the user was using the machine.
    last_activity_ms: Option<i64>,
    last_tick_ms: Option<i64>,
}

impl Presence {
    pub fn view(&self) -> View {
        let away = if self.asleep.is_some() {
            Some(AwayReason::Sleep)
        } else if self.screens_off.is_some() {
            Some(AwayReason::DisplayOff)
        } else if self.locked {
            Some(AwayReason::Locked)
        } else if let Some(r) = self
            .front
            .as_ref()
            .and_then(|f| away_app_reason(&f.bundle_id))
        {
            Some(r)
        } else if self.idle {
            Some(AwayReason::Idle)
        } else {
            None
        };
        View {
            front: self.front.clone(),
            away,
            halted: self.powering_off.is_some(),
        }
    }

    fn step(&self, at: Timestamp) -> Step {
        Step {
            at,
            view: self.view(),
        }
    }

    fn note_activity(&mut self, ms: i64) {
        self.last_activity_ms = Some(self.last_activity_ms.map_or(ms, |l| l.max(ms)));
    }

    /// Apply one event at `now`. Returns the resulting state(s), each with the
    /// time it takes effect.
    pub fn on_event(&mut self, ev: &SystemEvent, now: Timestamp, s: &Settings) -> Vec<Step> {
        match ev {
            SystemEvent::FrontApp { bundle_id, name } => {
                let same = self.front.as_ref().is_some_and(|f| {
                    f.bundle_id == *bundle_id && (!bundle_id.is_empty() || f.name == *name)
                });
                if same {
                    return Vec::new();
                }
                if away_app_reason(bundle_id).is_none() {
                    // A real app came to the front: the user is here. Nothing
                    // but the lock screen can be in front while locked, so this
                    // also recovers from a missed unlock notification.
                    self.idle = false;
                    self.locked = false;
                    self.powering_off = None;
                    self.note_activity(now.utc_ms);
                }
                self.front = Some(FrontApp {
                    bundle_id: bundle_id.clone(),
                    name: name.clone(),
                });
                vec![self.step(now)]
            }
            SystemEvent::IdleTick {
                secs_since_input,
                display_assertions,
            } => self.on_tick(now, *secs_since_input, !display_assertions.is_empty(), s),
            SystemEvent::WillSleep => {
                self.asleep = Some(now.utc_ms);
                self.last_tick_ms = None;
                vec![self.step(now)]
            }
            SystemEvent::DidWake => {
                self.asleep = None;
                self.screens_off = None;
                self.idle = false;
                self.note_activity(now.utc_ms);
                self.last_tick_ms = Some(now.utc_ms);
                vec![self.step(now)]
            }
            SystemEvent::ScreensSlept => {
                self.screens_off = Some(now.utc_ms);
                vec![self.step(now)]
            }
            SystemEvent::ScreensWoke => {
                self.screens_off = None;
                self.idle = false;
                self.note_activity(now.utc_ms);
                vec![self.step(now)]
            }
            SystemEvent::ScreenLocked => {
                self.locked = true;
                vec![self.step(now)]
            }
            SystemEvent::ScreenUnlocked => {
                self.locked = false;
                self.idle = false;
                self.note_activity(now.utc_ms);
                vec![self.step(now)]
            }
            SystemEvent::WillPowerOff => {
                self.powering_off = Some(now.utc_ms);
                vec![self.step(now)]
            }
            SystemEvent::Paused => vec![self.step(now)],
            SystemEvent::Resumed => {
                // Resuming from the tray is use. If a timed pause expired while
                // the user was away, the next tick backdates to this moment and
                // the resulting zero-length session is dropped.
                self.idle = false;
                self.note_activity(now.utc_ms);
                vec![self.step(now)]
            }
        }
    }

    fn on_tick(&mut self, now: Timestamp, secs: f64, assertion: bool, s: &Settings) -> Vec<Step> {
        let mut steps = Vec::new();
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        let last_input = now.utc_ms - (secs * MS_PER_SEC as f64) as i64;

        // Missed sleep: the tick timer stopped for longer than TICK_GAP_MS.
        let mut floor = i64::MIN;
        let mut gap = false;
        if let Some(last) = self.last_tick_ms
            && now.utc_ms - last > TICK_GAP_MS
            && self.asleep.is_none()
        {
            let gap_start = last
                .max(self.last_activity_ms.unwrap_or(last))
                .min(now.utc_ms);
            self.asleep = Some(gap_start);
            steps.push(self.step(now.with_utc_ms(gap_start)));
            floor = gap_start;
            gap = true;
        }
        self.last_tick_ms = Some(now.utc_ms);

        let awi = self
            .front
            .as_ref()
            .is_some_and(|f| s.is_active_without_input(&f.key()));
        let timeout = if awi {
            s.active_without_input_timeout_secs
        } else {
            s.idle_threshold_secs
        };

        if secs < f64::from(timeout) {
            // Recent input. Input made after the machine slept, the display
            // slept or power-off began also proves those are over (covers a
            // missed wake / cancelled shutdown notification).
            self.note_activity(last_input);
            let mut woke = std::mem::take(&mut self.idle);
            for state in [
                &mut self.asleep,
                &mut self.screens_off,
                &mut self.powering_off,
            ] {
                if state.is_some_and(|since| last_input > since) {
                    *state = None;
                    woke = true;
                }
            }
            let at = if woke {
                now.with_utc_ms(last_input.max(floor).min(now.utc_ms))
            } else {
                now
            };
            steps.push(self.step(at));
        } else if assertion {
            // Something (video, presentation) keeps the display awake.
            self.note_activity(now.utc_ms);
            self.idle = false;
            if gap {
                self.asleep = None;
            }
            steps.push(self.step(now));
        } else {
            if gap {
                self.asleep = None;
            }
            let evidence = last_input
                .max(self.last_activity_ms.unwrap_or(i64::MIN))
                .max(floor);
            if self.idle {
                steps.push(self.step(now));
            } else if awi {
                let end = evidence + i64::from(timeout) * MS_PER_SEC;
                if end > now.utc_ms {
                    // Non-input evidence (e.g. a programmatic app switch) is
                    // still within the timeout: not idle yet.
                    steps.push(self.step(now));
                } else {
                    self.idle = true;
                    steps.push(self.step(now.with_utc_ms(end.max(floor))));
                }
            } else {
                self.idle = true;
                steps.push(self.step(now.with_utc_ms(evidence.min(now.utc_ms))));
            }
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::{Clock, ManualClock};

    fn tick(secs: f64) -> SystemEvent {
        SystemEvent::IdleTick {
            secs_since_input: secs,
            display_assertions: vec![],
        }
    }
    fn app(b: &str) -> SystemEvent {
        SystemEvent::FrontApp {
            bundle_id: b.into(),
            name: b.into(),
        }
    }

    #[test]
    fn idle_is_backdated_to_last_input() {
        let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, 0);
        let s = Settings::default();
        let mut p = Presence::default();
        p.on_event(&app("a"), c.now(), &s);
        c.advance_secs(240);
        let steps = p.on_event(&tick(200.0), c.now(), &s);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].view.away, Some(AwayReason::Idle));
        assert_eq!(steps[0].at.utc_ms, c.now().utc_ms - 200_000);
    }

    #[test]
    fn duplicate_front_app_is_ignored() {
        let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, 0);
        let s = Settings::default();
        let mut p = Presence::default();
        assert_eq!(p.on_event(&app("a"), c.now(), &s).len(), 1);
        assert!(p.on_event(&app("a"), c.now(), &s).is_empty());
    }

    #[test]
    fn missing_bundle_id_uses_exe_name() {
        let f = FrontApp {
            bundle_id: String::new(),
            name: "java".into(),
        };
        assert_eq!(f.key(), "exe:java");
    }
}

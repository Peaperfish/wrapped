//! End-to-end tests: real engine, real collectors, SQLite, fake clock.
//! No OS involved: events are fed by hand exactly as the platform layer would.

use std::path::PathBuf;
use std::sync::Arc;

use wrapped_core::collectors;
use wrapped_core::settings::Pause;
use wrapped_core::store::SessionRow;
use wrapped_core::time::{MS_PER_DAY, MS_PER_MIN};
use wrapped_core::{DisplayAssertion, Engine, ManualClock, Store, SystemEvent, Timestamp};

const CST: i32 = 480; // UTC+8

struct H {
    e: Engine,
    c: ManualClock,
}

fn engine_at(store: Store, c: &ManualClock) -> Engine {
    Engine::new(store, Arc::new(c.clone()), collectors::builtin()).unwrap()
}

impl H {
    /// 2026-11-16 09:00:00 local, UTC+8, in-memory database.
    fn new() -> Self {
        Self::at(9, 0, 0, CST)
    }
    fn at(h: u32, m: u32, s: u32, off: i32) -> Self {
        let c = ManualClock::at_local(2026, 11, 16, h, m, s, off);
        let e = engine_at(Store::open_in_memory().unwrap(), &c);
        Self { e, c }
    }
    fn ev(&mut self, ev: SystemEvent) {
        self.e.handle(ev).unwrap();
    }
    fn app(&mut self, bundle: &str) {
        self.ev(SystemEvent::FrontApp {
            bundle_id: bundle.into(),
            name: bundle.rsplit('.').next().unwrap().into(),
        });
    }
    fn tick(&mut self, secs_since_input: f64) {
        self.tick_with(secs_since_input, &[]);
    }
    /// Tick with display assertions: `(holder, held by the front app)`.
    fn tick_with(&mut self, secs_since_input: f64, holders: &[(&str, bool)]) {
        self.ev(SystemEvent::IdleTick {
            secs_since_input,
            display_assertions: holders
                .iter()
                .map(|&(h, front)| DisplayAssertion::new(h, front))
                .collect(),
        });
    }
    fn adv(&self, secs: i64) {
        self.c.advance_secs(secs);
    }
    /// Advance minute by minute, ticking with "input just now".
    fn active_for(&mut self, minutes: i64) {
        for _ in 0..minutes {
            self.adv(60);
            self.tick(1.0);
        }
    }
    fn flush(&mut self) {
        self.e.flush().unwrap();
    }
    fn sessions(&mut self, collector: &str) -> Vec<(String, String, String, bool)> {
        self.flush();
        rows(
            self.e
                .store()
                .sessions(Some(collector), i64::MIN, i64::MAX)
                .unwrap(),
        )
    }
    fn metric(&self, date: &str, collector: &str, metric: &str, key: &str) -> Option<f64> {
        let d = date.parse().unwrap();
        self.e
            .store()
            .daily_metrics(d)
            .unwrap()
            .into_iter()
            .find(|r| r.collector == collector && r.metric == metric && r.key == key)
            .map(|r| r.value)
    }
}

/// Local wall-clock "HH:MM:SS" in the timestamp's own offset.
fn hms(t: Timestamp) -> String {
    let s = t.local_ms().rem_euclid(MS_PER_DAY) / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn rows(v: Vec<SessionRow>) -> Vec<(String, String, String, bool)> {
    v.into_iter()
        .map(|r| (r.key, hms(r.start), hms(r.end), r.open))
        .collect()
}

fn s(key: &str, a: &str, b: &str, open: bool) -> (String, String, String, bool) {
    (key.into(), a.into(), b.into(), open)
}

const AU: &str = "app_usage";
const AI: &str = "active_idle";
const DAY: &str = "2026-11-16";

#[test]
fn app_switch_sequence_and_metrics() {
    let mut h = H::new();
    h.app("com.a");
    h.active_for(5);
    h.app("com.b");
    h.active_for(2);
    h.app("com.c");
    h.active_for(1);
    h.app("com.a");
    h.adv(30);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:05:00", false),
            s("com.b", "09:05:00", "09:07:00", false),
            s("com.c", "09:07:00", "09:08:00", false),
            s("com.a", "09:08:00", "09:08:30", true),
        ]
    );
    assert_eq!(h.metric(DAY, AU, "screen_time_secs", ""), Some(510.0));
    assert_eq!(h.metric(DAY, AU, "app_secs", "com.a"), Some(330.0));
    assert_eq!(h.metric(DAY, AU, "app_secs", "com.b"), Some(120.0));
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(3.0));
    assert_eq!(h.metric(DAY, AU, "active_secs_by_hour", "09"), Some(510.0));
    assert!(h.sessions(AI).is_empty());
}

const OWN: &str = "com.own.app";

#[test]
fn own_app_front_event_is_ignored() {
    let mut h = H::new();
    h.e.set_own_app_id(OWN);
    h.app("com.a");
    h.active_for(2);
    h.app(OWN);
    h.active_for(1);
    h.app("com.a");
    h.active_for(1);
    h.adv(30);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:04:30", true)]
    );
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(0.0));
    assert_eq!(h.metric(DAY, AU, "app_secs", OWN), None);
}

#[test]
fn own_app_then_other_app_ends_session_at_the_other_app() {
    let mut h = H::new();
    h.e.set_own_app_id(OWN);
    h.app("com.a");
    h.active_for(2);
    h.app(OWN);
    h.active_for(1);
    h.app("com.b");
    h.adv(30);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:03:00", false),
            s("com.b", "09:03:00", "09:03:30", true),
        ]
    );
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(1.0));
}

#[test]
fn duplicate_front_app_events_do_not_split() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(10);
    h.app("com.a");
    h.adv(10);
    h.app("com.a");
    h.adv(10);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:00:30", true)]
    );
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(0.0));
}

#[test]
fn idle_is_backdated_to_last_input_and_return_restarts() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.tick(20.0); // input at 09:00:40
    h.adv(60);
    h.tick(80.0);
    h.adv(60);
    h.tick(140.0);
    h.adv(60);
    h.tick(200.0); // 09:04:00: idle, last input 09:00:40
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:00:40", false)]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:00:40", "09:04:00", true)]
    );
    // Back at the keyboard; the 09:10 tick sees input 5 s ago. The ticks in
    // between keep the gap below TICK_GAP_MS (a longer gap counts as sleep).
    for _ in 0..5 {
        h.adv(60);
        h.tick(300.0);
    }
    h.adv(60);
    h.tick(5.0);
    h.adv(60);
    h.tick(1.0);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:00:40", false),
            s("com.a", "09:09:55", "09:11:00", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:00:40", "09:09:55", false)]
    );
    assert_eq!(h.metric(DAY, AI, "away_secs", ""), Some(555.0));
    assert_eq!(h.metric(DAY, AI, "away_periods", ""), Some(1.0));
    assert_eq!(h.metric(DAY, AU, "screen_time_secs", ""), Some(105.0));
    // Same app on both sides of an idle gap is not an app switch.
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(0.0));
}

#[test]
fn idle_backdate_never_goes_before_an_app_switch() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(30);
    h.app("com.b"); // 09:00:30; switching apps is evidence of use
    h.adv(210);
    h.tick(240.0); // last "input" 09:00:00, but the switch was later
    // b's session collapses to zero length and is dropped.
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:00:30", false)]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:00:30", "09:04:00", true)]
    );
}

#[test]
fn display_assertion_keeps_user_active() {
    let mut h = H::new();
    h.app("com.apple.QuickTimePlayerX");
    for m in 1..=20 {
        h.adv(60);
        h.tick_with(60.0 * m as f64, &[("QuickTime Player", true)]);
    }
    // 09:20: video still playing, 20 min without input: still one open session.
    assert_eq!(
        h.sessions(AU),
        vec![s(
            "com.apple.QuickTimePlayerX",
            "09:00:00",
            "09:20:00",
            true
        )]
    );
    assert!(h.sessions(AI).is_empty());
    // Video ends; next tick has no holder: idle from the last tick that had one.
    h.adv(60);
    h.tick(1260.0);
    assert_eq!(
        h.sessions(AU),
        vec![s(
            "com.apple.QuickTimePlayerX",
            "09:00:00",
            "09:20:00",
            false
        )]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:20:00", "09:21:00", true)]
    );
}

const CAFFEINATE: (&str, bool) = ("caffeinate[49222] caffeinate command-line tool", false);

/// 2026-11-17: `caffeinate -d` started from a terminal kept the display awake
/// while nobody was there. A background holder is not evidence of use: idle
/// is backdated to the last input as if there were no assertion.
#[test]
fn background_display_assertion_does_not_keep_user_active() {
    let mut h = H::new();
    h.app("com.mitchellh.ghostty");
    // Last input 09:00:30, then nobody for 78 minutes.
    for m in 1..=78 {
        h.adv(60);
        h.tick_with(60.0 * m as f64 - 30.0, &[CAFFEINATE]);
    }
    assert_eq!(
        h.sessions(AU),
        vec![s("com.mitchellh.ghostty", "09:00:00", "09:00:30", false)]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:00:30", "10:18:00", true)]
    );
}

/// Same with the terminal on the active-without-input list: its timeout runs
/// from the last input, not from the last tick that saw the assertion.
#[test]
fn background_display_assertion_does_not_extend_active_without_input_timeout() {
    let mut h = H::new();
    h.e.store()
        .set_setting("active_without_input.apps", r#"["com.mitchellh.ghostty"]"#)
        .unwrap();
    h.tick(0.0); // reload settings
    h.app("com.mitchellh.ghostty");
    for m in 1..=78 {
        h.adv(60);
        h.tick_with(60.0 * m as f64 - 30.0, &[CAFFEINATE]);
    }
    // Last input 09:00:30 + 1800 s timeout.
    assert_eq!(
        h.sessions(AU),
        vec![s("com.mitchellh.ghostty", "09:00:00", "09:30:30", false)]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:30:30", "10:18:00", true)]
    );
}

/// A front-app assertion (video playing in front) keeps the user present for
/// as long as it is held; once released, an active-without-input app still
/// gets its timeout, counted from the last tick that saw the assertion.
#[test]
fn front_app_display_assertion_keeps_active_then_uses_its_timeout() {
    let mut h = H::new();
    h.e.store()
        .set_setting(
            "active_without_input.apps",
            r#"["com.apple.QuickTimePlayerX"]"#,
        )
        .unwrap();
    h.tick(0.0); // reload settings
    h.app("com.apple.QuickTimePlayerX");
    let video = ("QuickTime Player[700] Playing video", true);
    for m in 1..=120 {
        h.adv(60);
        h.tick_with(60.0 * m as f64, &[video, CAFFEINATE]);
    }
    // 11:00: two hours of video without input, still one open session.
    assert_eq!(
        h.sessions(AU),
        vec![s(
            "com.apple.QuickTimePlayerX",
            "09:00:00",
            "11:00:00",
            true
        )]
    );
    assert!(h.sessions(AI).is_empty());
    // Video stops at 11:00; only the background holder is left.
    for m in 121..=149 {
        h.adv(60);
        h.tick_with(60.0 * m as f64, &[CAFFEINATE]);
    }
    assert!(h.sessions(AI).is_empty(), "within the 1800 s timeout");
    for m in 150..=151 {
        h.adv(60);
        h.tick_with(60.0 * m as f64, &[CAFFEINATE]);
    }
    assert_eq!(
        h.sessions(AU),
        vec![s(
            "com.apple.QuickTimePlayerX",
            "09:00:00",
            "11:30:00",
            false
        )]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "11:30:00", "11:31:00", true)]
    );
}

/// The front app's assertion comes and goes while a background one stays:
/// only the front one counts, each time.
#[test]
fn display_assertion_switching_between_front_app_and_background() {
    let mut h = H::new();
    h.app("com.apple.QuickTimePlayerX");
    let video = ("QuickTime Player[700] Playing video", true);
    let mut m = 0;
    let mut tick = |h: &mut H, holders: &[(&str, bool)]| {
        m += 1;
        h.adv(60);
        h.tick_with(60.0 * m as f64, holders);
    };
    for _ in 1..=10 {
        tick(&mut h, &[CAFFEINATE, video]); // 09:01..09:10 playing in front
    }
    tick(&mut h, &[CAFFEINATE]); // 09:11 paused: idle from 09:10
    tick(&mut h, &[CAFFEINATE]); // 09:12
    tick(&mut h, &[video, CAFFEINATE]); // 09:13 playing again
    tick(&mut h, &[video, CAFFEINATE]); // 09:14
    tick(&mut h, &[video, CAFFEINATE]); // 09:15
    tick(&mut h, &[CAFFEINATE]); // 09:16 stopped: idle from 09:15
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.apple.QuickTimePlayerX", "09:00:00", "09:10:00", false),
            s("com.apple.QuickTimePlayerX", "09:13:00", "09:15:00", false),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![
            s("idle", "09:10:00", "09:13:00", false),
            s("idle", "09:15:00", "09:16:00", true),
        ]
    );
}

#[test]
fn active_without_input_app_uses_longer_timeout() {
    let mut h = H::new();
    h.e.store()
        .set_setting("active_without_input.apps", r#"["com.apple.Preview"]"#)
        .unwrap();
    h.tick(0.0); // reload settings
    h.app("com.apple.Preview");
    h.adv(60);
    h.tick(25.0); // last input 09:00:35
    for m in 2..=30 {
        h.adv(60);
        h.tick(60.0 * m as f64 - 25.0);
    }
    // 09:30: 29m25s without input in a reading app: still active.
    assert_eq!(
        h.sessions(AU),
        vec![s("com.apple.Preview", "09:00:00", "09:30:00", true)]
    );
    // 09:31 tick sees 30m25s: the 30-minute timeout ran out at 09:30:35.
    // The session ends there: not at the tick (09:31), not at last input.
    h.adv(60);
    h.tick(1825.0);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.apple.Preview", "09:00:00", "09:30:35", false)]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("idle", "09:30:35", "09:31:00", true)]
    );

    // An ordinary app goes idle at the normal 180 s threshold, backdated to
    // the last input.
    let mut h = H::new();
    h.e.store()
        .set_setting("active_without_input.apps", r#"["com.apple.Preview"]"#)
        .unwrap();
    h.app("com.other");
    h.adv(60);
    h.tick(30.0); // input at 09:00:30
    for m in 2..=4 {
        h.adv(60);
        h.tick(60.0 * m as f64 - 30.0);
    }
    assert_eq!(
        h.sessions(AU),
        vec![s("com.other", "09:00:00", "09:00:30", false)]
    );
}

#[test]
fn awi_timeout_counts_from_last_evidence_not_last_input() {
    // Switching to the reading app is evidence of use even with no input since.
    let mut h = H::new();
    h.e.store()
        .set_setting("active_without_input.apps", r#"["com.apple.Preview"]"#)
        .unwrap();
    h.tick(0.0);
    h.app("com.a");
    h.adv(120);
    h.app("com.apple.Preview"); // 09:02, last input was 09:00
    for m in 3..=32 {
        h.adv(60);
        h.tick(60.0 * m as f64);
    }
    // 09:32: 32 min since input, but only 30 since the switch: idle at 09:32:00.
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:02:00", false),
            s("com.apple.Preview", "09:02:00", "09:32:00", false),
        ]
    );
}

#[test]
fn awi_app_with_occasional_input_keeps_the_whole_session() {
    let mut h = H::new();
    h.e.store()
        .set_setting("active_without_input.apps", r#"["com.apple.Preview"]"#)
        .unwrap();
    h.tick(0.0);
    h.app("com.apple.Preview");
    // Scroll once every 20 minutes for an hour.
    for m in 1..=60 {
        h.adv(60);
        h.tick(60.0 * f64::from(m % 20));
    }
    assert_eq!(
        h.sessions(AU),
        vec![s("com.apple.Preview", "09:00:00", "10:00:00", true)]
    );
}

#[test]
fn sleep_ends_session_and_wake_restarts() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(300);
    let r = h.e.handle(SystemEvent::WillSleep).unwrap();
    assert!(r.is_some(), "WillSleep flushes immediately");
    h.adv(8 * 3600); // overnight
    h.ev(SystemEvent::DidWake);
    h.adv(60);
    h.tick(2.0);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:05:00", false),
            s("com.a", "17:05:00", "17:06:00", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("sleep", "09:05:00", "17:05:00", false)]
    );
    assert_eq!(
        h.metric(DAY, AI, "away_secs_by_reason", "sleep"),
        Some(28800.0)
    );
}

#[test]
fn idle_that_turns_into_sleep_is_split_at_the_sleep_moment() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.tick(20.0); // input at 09:00:40
    h.adv(60);
    h.tick(80.0);
    h.adv(60);
    h.tick(140.0);
    h.adv(60);
    h.tick(200.0); // 09:04:00: idle since 09:00:40
    for _ in 0..6 {
        h.adv(60);
        h.tick(300.0); // 09:10:00, still idle
    }
    h.ev(SystemEvent::WillSleep);
    h.adv(3600);
    h.ev(SystemEvent::DidWake);
    h.adv(60);
    h.tick(2.0);
    assert_eq!(
        h.sessions(AI),
        vec![
            s("idle", "09:00:40", "09:10:00", false),
            s("sleep", "09:10:00", "10:10:00", false),
        ]
    );
    assert_eq!(h.metric(DAY, AI, "away_secs_by_reason", "idle"), Some(560.0));
    assert_eq!(h.metric(DAY, AI, "away_secs_by_reason", "sleep"), Some(3600.0));
    // One continuous away period: the split is not a new "time away".
    assert_eq!(h.metric(DAY, AI, "away_periods", ""), Some(1.0));
}

/// macOS runs the minute timer during dark wakes (maintenance wakes) without
/// sending a wake notification. Those ticks report stale input (from before
/// the sleep) and must neither end the sleep period nor trigger a flush.
#[test]
fn dark_wake_ticks_keep_sleeping_and_skip_periodic_flush() {
    let mut h = H::new();
    h.app("com.a");
    h.active_for(5); // 09:05
    let r = h.e.handle(SystemEvent::WillSleep).unwrap();
    assert!(r.is_some(), "WillSleep flushes immediately");
    assert!(h.e.is_asleep());
    let mut asleep_secs = 0;
    for at_min in [5, 60, 180] {
        let to = at_min * 60;
        h.adv(to - asleep_secs);
        asleep_secs = to;
        // Last input was just before WillSleep: older than the time asleep.
        let r =
            h.e.handle(SystemEvent::IdleTick {
                secs_since_input: (asleep_secs + 30) as f64,
                display_assertions: vec![],
            })
            .unwrap();
        assert!(r.is_none(), "no flush during a dark wake (+{at_min} min)");
        assert!(h.e.is_asleep());
    }
    h.adv(4 * 3600 - asleep_secs); // DidWake at 13:05
    let r = h.e.handle(SystemEvent::DidWake).unwrap();
    assert!(r.is_some(), "the sleep period is written at wake");
    assert!(!h.e.is_asleep());
    h.app("com.b");
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:05:00", false),
            s("com.b", "13:05:00", "13:06:00", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("sleep", "09:05:00", "13:05:00", false)]
    );
    assert_eq!(h.metric(DAY, AU, "app_secs", "com.a"), Some(300.0));
}

#[test]
fn input_after_sleep_without_did_wake_resumes_flushing() {
    let mut h = H::new();
    h.app("com.a");
    h.ev(SystemEvent::WillSleep);
    h.adv(3600);
    h.tick(5.0); // input 5 s ago: the wake notification was missed
    assert!(!h.e.is_asleep());
}

#[test]
fn lock_ends_session_and_unlock_restarts() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(120);
    h.ev(SystemEvent::ScreenLocked);
    h.app("com.apple.loginwindow");
    h.adv(60);
    h.tick(50.0); // typing the password is input, but still locked
    h.adv(5);
    h.ev(SystemEvent::ScreenUnlocked);
    h.app("com.a");
    h.adv(55);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:02:00", false),
            s("com.a", "09:03:05", "09:04:00", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("locked", "09:02:00", "09:03:05", false)]
    );
    assert!(
        h.sessions(AU)
            .iter()
            .all(|r| r.0 != "com.apple.loginwindow")
    );
}

#[test]
fn display_sleep_ends_session() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.ev(SystemEvent::ScreensSlept);
    h.adv(60);
    h.tick(70.0);
    h.adv(60);
    h.ev(SystemEvent::ScreensWoke);
    h.adv(30);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.a", "09:03:00", "09:03:30", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("display_off", "09:01:00", "09:03:00", false)]
    );
}

#[test]
fn missed_screens_woke_is_recovered_by_input() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.ev(SystemEvent::ScreensSlept);
    h.adv(120);
    h.tick(10.0); // input 10 s ago means the display is on again
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.a", "09:02:50", "09:04:00", true),
        ]
    );
}

#[test]
fn loginwindow_and_screensaver_are_away_never_app_time() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.app("com.apple.loginwindow");
    h.adv(600);
    h.tick(10.0); // input on the lock screen does not make it app time
    h.app("com.a");
    h.adv(60);
    h.app("com.apple.ScreenSaver.Engine");
    h.adv(60);
    h.app("com.a");
    h.adv(60);
    let au = h.sessions(AU);
    assert!(au.iter().all(|r| r.0 == "com.a"), "{au:?}");
    assert_eq!(
        au,
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.a", "09:11:00", "09:12:00", false),
            s("com.a", "09:13:00", "09:14:00", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![
            s("locked", "09:01:00", "09:11:00", false),
            s("screensaver", "09:12:00", "09:13:00", false),
        ]
    );
}

#[test]
fn excluded_apps_leave_a_gap_and_no_trace() {
    let mut h = H::new();
    h.e.store()
        .set_setting("exclude.apps", r#"["com.secret.app"]"#)
        .unwrap();
    h.tick(0.0);
    h.app("com.a");
    h.adv(60);
    h.app("com.secret.app");
    h.adv(120);
    h.tick(5.0);
    h.app("com.b");
    h.adv(60);
    h.flush();
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.b", "09:03:00", "09:04:00", true),
        ]
    );
    let conn = h.e.store().conn();
    for (table, col) in [
        ("sessions", "key"),
        ("daily_metrics", "key"),
        ("settings", "key"),
    ] {
        let n: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {col} LIKE '%secret%'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    // a -> secret -> b is not an a -> b switch (there is a gap).
    assert_eq!(h.metric(DAY, AU, "app_switches", ""), Some(0.0));
}

#[test]
fn excluding_the_current_app_closes_its_session_at_next_tick() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.e.store()
        .set_setting("exclude.apps", r#"["com.a"]"#)
        .unwrap();
    h.tick(1.0);
    h.adv(60);
    h.tick(1.0);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:01:00", false)]
    );
}

#[test]
fn midnight_split_for_daily_metrics() {
    let mut h = H::at(23, 50, 0, CST);
    h.app("com.a");
    h.active_for(20); // until 00:10
    h.app("com.b");
    h.adv(60);
    h.flush();
    assert_eq!(
        h.sessions(AU)[0],
        s("com.a", "23:50:00", "00:10:00", false),
        "stored unsplit"
    );
    assert_eq!(
        h.metric("2026-11-16", AU, "screen_time_secs", ""),
        Some(600.0)
    );
    assert_eq!(
        h.metric("2026-11-16", AU, "active_secs_by_hour", "23"),
        Some(600.0)
    );
    assert_eq!(
        h.metric("2026-11-17", AU, "screen_time_secs", ""),
        Some(660.0)
    );
    assert_eq!(h.metric("2026-11-17", AU, "app_secs", "com.a"), Some(600.0));
    assert_eq!(
        h.metric("2026-11-17", AU, "active_secs_by_hour", "00"),
        Some(660.0)
    );
    // The continuation after midnight is not a switch; a -> b is.
    assert_eq!(h.metric("2026-11-16", AU, "app_switches", ""), Some(0.0));
    assert_eq!(h.metric("2026-11-17", AU, "app_switches", ""), Some(1.0));
}

#[test]
fn utc_offset_change_mid_session_splits_and_keeps_wall_clock_duration() {
    // Flying-ish: UTC+8 -> UTC+9 while using the same app, crossing local midnight.
    let mut h = H::at(23, 30, 0, CST);
    h.app("com.a");
    h.active_for(20); // 23:50 local (+8)
    h.c.set_offset_min(540); // now 00:50 local (+9), same instant
    h.active_for(20); // 01:10 local (+9)
    h.flush();
    let rows = h.e.store().sessions(Some(AU), i64::MIN, i64::MAX).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].start.offset_min, 480);
    assert_eq!(rows[1].start.offset_min, 540);
    let total: i64 = rows.iter().map(|r| r.end.utc_ms - r.start.utc_ms).sum();
    assert_eq!(total, 40 * MS_PER_MIN, "durations follow the wall clock");
    // First part on the 16th (23:30-23:51 at +8, split at the first +9 tick),
    // second part on the 17th (00:51-01:10 at +9).
    let d16 = h.metric("2026-11-16", AU, "screen_time_secs", "").unwrap();
    let d17 = h.metric("2026-11-17", AU, "screen_time_secs", "").unwrap();
    assert_eq!(d16 + d17, 2400.0);
    assert_eq!(d16, 1260.0);
    assert_eq!(h.metric("2026-11-17", AU, "app_switches", ""), Some(0.0));
}

#[test]
fn dst_fall_back_mid_session() {
    // US fall back: 01:59 EDT (-4) -> 01:00 EST (-5). One hour of local time repeats.
    let mut h = H::at(1, 30, 0, -240);
    h.app("com.a");
    h.active_for(30); // 02:00 EDT == 01:00 EST
    h.c.set_offset_min(-300);
    h.active_for(30);
    let all = h.sessions(AU);
    assert_eq!(
        all,
        vec![
            // Ends 02:01 EDT, stored with the offset in force then: 01:01 EST.
            s("com.a", "01:30:00", "01:01:00", false),
            s("com.a", "01:01:00", "01:30:00", true),
        ]
    );
    // An hour of wall-clock time, all on the same local day.
    assert_eq!(h.metric(DAY, AU, "screen_time_secs", ""), Some(3600.0));
}

#[test]
fn pause_and_resume() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    let now = h.e.now();
    h.e.set_pause(Pause::for_one_hour(now)).unwrap();
    assert!(h.e.is_paused());
    h.adv(60);
    h.app("com.b"); // recorded nowhere while paused
    h.adv(600);
    h.tick(900.0); // idle while paused: no away period either
    h.adv(60);
    h.e.set_pause(Pause::Off).unwrap();
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.b", "09:13:00", "09:14:00", true),
        ]
    );
    assert!(h.sessions(AI).is_empty());
}

#[test]
fn collector_turned_off_while_paused_records_again_after_resume() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.e.set_pause(Pause::Indefinite).unwrap();
    h.e.disable_collector(AU, true).unwrap();
    h.e.disable_collector(AI, true).unwrap();
    h.adv(60);
    // Resumed while off: the stopped collectors never see `Resumed`.
    h.e.set_pause(Pause::Off).unwrap();
    h.e.enable_collector(AU).unwrap();
    h.e.enable_collector(AI).unwrap();
    h.adv(60);
    h.app("com.b");
    h.adv(60);
    h.tick(1.0);
    h.adv(600);
    h.tick(600.0); // idle: an away period
    let au = h.sessions(AU);
    assert!(
        au.iter().any(|r| r.0 == "com.b"),
        "app_usage still thinks it is paused: {au:?}"
    );
    assert!(
        !h.sessions(AI).is_empty(),
        "active_idle still thinks it is paused"
    );
}

#[test]
fn timed_pause_expires_on_tick_and_is_persisted() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    let now = h.e.now();
    h.e.set_pause(Pause::for_one_hour(now)).unwrap();
    for _ in 0..60 {
        h.adv(60);
        h.tick(1.0);
    }
    assert!(!h.e.is_paused());
    assert_eq!(h.e.store().load_settings().unwrap().0.pause, Pause::Off);
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:01:00", false),
            s("com.a", "10:01:00", "10:02:00", true),
        ]
    );
}

#[test]
fn pause_from_another_process_is_picked_up_on_tick() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.e.store().set_setting("pause", "indefinite").unwrap();
    h.tick(1.0);
    assert!(h.e.is_paused());
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:00:00", "09:01:00", false)]
    );
}

#[test]
fn paused_state_survives_restart() {
    let path = temp_db("pause_restart");
    let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, CST);
    {
        let mut e = engine_at(Store::open(&path).unwrap(), &c);
        e.set_pause(Pause::Indefinite).unwrap();
        e.shutdown().unwrap();
    }
    let mut e = engine_at(Store::open(&path).unwrap(), &c);
    assert!(e.is_paused());
    e.handle(SystemEvent::FrontApp {
        bundle_id: "com.a".into(),
        name: "a".into(),
    })
    .unwrap();
    c.advance_secs(60);
    e.flush().unwrap();
    assert!(
        e.store()
            .sessions(None, i64::MIN, i64::MAX)
            .unwrap()
            .is_empty()
    );
    cleanup(&path);
}

#[test]
fn flushes_every_ten_minutes_and_upserts_open_session() {
    let mut h = H::new();
    h.app("com.a");
    let mut flushes = 0;
    for _ in 0..25 {
        h.adv(60);
        if h.e
            .handle(SystemEvent::IdleTick {
                secs_since_input: 1.0,
                display_assertions: vec![],
            })
            .unwrap()
            .is_some()
        {
            flushes += 1;
        }
    }
    assert_eq!(flushes, 2); // at 09:10 and 09:20
    let rows = rows(h.e.store().sessions(Some(AU), i64::MIN, i64::MAX).unwrap());
    assert_eq!(rows, vec![s("com.a", "09:00:00", "09:20:00", true)]);
}

#[test]
fn crash_recovery_closes_open_session_at_last_flush() {
    let path = temp_db("crash");
    let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, CST);
    {
        let mut e = engine_at(Store::open(&path).unwrap(), &c);
        e.handle(SystemEvent::FrontApp {
            bundle_id: "com.a".into(),
            name: "a".into(),
        })
        .unwrap();
        c.advance_secs(600);
        e.flush().unwrap(); // open session written with end = 09:10
        c.advance_secs(300);
        // Crash: the engine is dropped without shutdown; 09:10-09:15 is lost.
        std::mem::forget(e);
    }
    c.advance_secs(60);
    let e = engine_at(Store::open(&path).unwrap(), &c);
    assert_eq!(e.recovered_sessions(), 1);
    let r = rows(e.store().sessions(Some(AU), i64::MIN, i64::MAX).unwrap());
    assert_eq!(r, vec![s("com.a", "09:00:00", "09:10:00", false)]);
    assert_eq!(
        e.store()
            .daily_metrics(DAY.parse().unwrap())
            .unwrap()
            .iter()
            .find(|m| m.metric == "screen_time_secs")
            .map(|m| m.value),
        Some(600.0)
    );
    cleanup(&path);
}

#[test]
fn shutdown_closes_sessions_and_flushes() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(90);
    h.e.shutdown().unwrap();
    let r = rows(h.e.store().sessions(Some(AU), i64::MIN, i64::MAX).unwrap());
    assert_eq!(r, vec![s("com.a", "09:00:00", "09:01:30", false)]);
}

#[test]
fn backdated_idle_after_a_flush_past_midnight_fixes_the_next_day() {
    let mut h = H::at(23, 57, 0, CST);
    h.app("com.a");
    h.adv(60);
    h.tick(10.0); // last input 23:57:50
    h.adv(300);
    h.flush(); // open session written up to 00:03 on the 17th
    assert_eq!(
        h.metric("2026-11-17", AU, "screen_time_secs", ""),
        Some(180.0)
    );
    h.tick(310.0); // idle, backdated to 23:57:50
    h.flush();
    assert_eq!(h.metric("2026-11-17", AU, "screen_time_secs", ""), None);
    assert_eq!(
        h.metric("2026-11-16", AU, "screen_time_secs", ""),
        Some(50.0)
    );
}

#[test]
fn missed_sleep_notification_detected_from_tick_gap() {
    let mut h = H::new();
    h.app("com.a");
    h.active_for(5); // last tick 09:05
    h.adv(3 * 3600); // no WillSleep, no ticks: the machine was asleep
    h.tick(30.0);
    h.adv(30);
    assert_eq!(
        h.sessions(AU),
        vec![
            s("com.a", "09:00:00", "09:05:00", false),
            s("com.a", "12:04:30", "12:05:30", true),
        ]
    );
    assert_eq!(
        h.sessions(AI),
        vec![s("sleep", "09:05:00", "12:04:30", false)]
    );
}

#[test]
fn power_off_ends_everything() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    assert!(h.e.handle(SystemEvent::WillPowerOff).unwrap().is_some());
    h.adv(5);
    h.e.shutdown().unwrap();
    assert_eq!(
        rows(h.e.store().sessions(None, i64::MIN, i64::MAX).unwrap()),
        vec![s("com.a", "09:00:00", "09:01:00", false)]
    );
}

#[test]
fn disable_collector_keep_or_delete() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(60);
    h.tick(1.0);
    h.adv(60);
    h.tick(400.0);
    h.adv(60);
    assert!(!h.sessions(AI).is_empty());

    h.e.disable_collector(AI, true).unwrap();
    assert_eq!(h.e.running_collectors(), vec![AU]);
    assert!(!h.sessions(AI).is_empty(), "kept");
    assert!(
        !h.e.store()
            .load_settings()
            .unwrap()
            .0
            .collector_enabled(AI, true)
    );

    h.e.enable_collector(AI).unwrap();
    h.e.disable_collector(AI, false).unwrap();
    assert!(h.sessions(AI).is_empty(), "deleted");
    assert!(h.metric(DAY, AI, "away_secs", "").is_none());
    assert!(!h.sessions(AU).is_empty(), "other collectors untouched");
}

#[test]
fn delete_collector_data_while_running() {
    let mut h = H::new();
    h.app("com.a");
    h.adv(600);
    h.flush();
    h.e.delete_collector_data(AU).unwrap();
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:10:00", "09:11:00", true)]
    );
    assert_eq!(h.metric(DAY, AU, "screen_time_secs", ""), Some(60.0));
}

#[test]
fn enabling_a_collector_mid_run_picks_up_the_current_app() {
    let mut h = H::new();
    h.e.disable_collector(AU, true).unwrap();
    h.app("com.a");
    h.adv(60);
    h.e.enable_collector(AU).unwrap();
    h.adv(60);
    assert_eq!(
        h.sessions(AU),
        vec![s("com.a", "09:01:00", "09:02:00", true)]
    );
}

fn temp_db(name: &str) -> PathBuf {
    let p = std::env::temp_dir()
        .join(format!("wrapped-core-tests-{}", std::process::id()))
        .join(format!("{name}.db"));
    cleanup(&p);
    p
}

fn cleanup(p: &std::path::Path) {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", p.display()));
    }
}

#[test]
fn excluded_app_is_redacted_in_debug_log() {
    let mut h = H::new();
    let log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = log.clone();
    h.e.set_logger(move |m| sink.lock().unwrap().push(m.to_string()));
    h.e.store()
        .set_setting("exclude.apps", r#"["com.secret.app"]"#)
        .unwrap();
    h.tick(0.0);
    h.app("com.secret.app");
    h.app("com.a");
    let log = log.lock().unwrap();
    assert!(log.iter().all(|l| !l.contains("secret")), "{log:?}");
    assert!(log.iter().any(|l| l.contains("<excluded app>")));
    assert!(log.iter().any(|l| l.contains("com.a")));
}

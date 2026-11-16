//! Milestone 0 spike: event-driven front-app tracking, idle check, sleep/wake.
//! No permissions needed (no Accessibility / Input Monitoring / Screen Recording).

use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};
use objc2_foundation::{NSNotification, NSRunLoop, NSTimer};

const IDLE_THRESHOLD_SECS: f64 = 180.0;
const IDLE_CHECK_INTERVAL_SECS: f64 = 60.0;

fn ts() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() % 86400;
    format!("{:02}:{:02}:{:02}Z", s / 3600, (s / 60) % 60, s % 60)
}

fn log(msg: &str) {
    println!("[{}] {}", ts(), msg);
}

fn describe(app: &NSRunningApplication) -> String {
    let name = app.localizedName().map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    let bid = app.bundleIdentifier().map(|n| n.to_string()).unwrap_or_else(|| "?".into());
    format!("{name} ({bid})")
}

fn idle_secs() -> f64 {
    // kCGAnyInputEventType = ~0: any keyboard/mouse/trackpad event.
    CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::CombinedSessionState,
        CGEventType(u32::MAX),
    )
}

fn report_idle() {
    let s = idle_secs();
    log(&format!(
        "idle: {:.0}s since last input -> {}",
        s,
        if s >= IDLE_THRESHOLD_SECS { "IDLE" } else { "active" }
    ));
}

fn main() {
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        let nc = ws.notificationCenter();

        if let Some(app) = ws.frontmostApplication() {
            log(&format!("front app (initial): {}", describe(&app)));
        }

        // App activation (event-driven; no polling).
        let on_activate = RcBlock::new(|n: NonNull<NSNotification>| {
            let n = n.as_ref();
            if let Some(info) = n.userInfo() {
                if let Some(obj) = info.objectForKey(NSWorkspaceApplicationKey) {
                    if let Ok(app) = obj.downcast::<NSRunningApplication>() {
                        log(&format!("front app: {}", describe(&app)));
                    }
                }
            }
        });
        let _t1 = nc.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &on_activate,
        );

        let on_sleep = RcBlock::new(|_: NonNull<NSNotification>| log("SLEEP (willSleep)"));
        let _t2 = nc.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillSleepNotification),
            None,
            None,
            &on_sleep,
        );
        let on_wake = RcBlock::new(|_: NonNull<NSNotification>| {
            log("WAKE (didWake)");
            report_idle();
        });
        let _t3 = nc.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            None,
            &on_wake,
        );

        // Once-a-minute idle check via run-loop timer (no busy loop).
        let on_timer = RcBlock::new(|_: NonNull<NSTimer>| report_idle());
        let _timer = NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            IDLE_CHECK_INTERVAL_SECS,
            true,
            &on_timer,
        );

        report_idle();
        NSRunLoop::currentRunLoop().run();
    }
}

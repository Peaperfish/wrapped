//! Milestone 0 spike: event-driven front-app tracking, idle check, sleep/wake,
//! display sleep/wake, and display-sleep-assertion check.
//! No permissions needed (no Accessibility / Input Monitoring / Screen Recording).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};
use objc2_foundation::{NSNotification, NSRunLoop, NSString, NSTimer};

const IDLE_THRESHOLD_SECS: f64 = 180.0;
const CHECK_INTERVAL_SECS: f64 = 60.0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    // Public IOKit power-management API; returns kIOReturnSuccess (0) and a +1 CFDictionary
    // of pid(CFNumber) -> CFArray of assertion CFDictionaries. No entitlement/permission.
    fn IOPMCopyAssertionsByProcess(out: *mut *mut c_void) -> i32;
}

static LAST_BUNDLE: Mutex<String> = Mutex::new(String::new());
static CYCLE_EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn ts() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as libc::time_t;
    unsafe {
        let mut u: libc::tm = std::mem::zeroed();
        let mut l: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&secs, &mut u);
        libc::localtime_r(&secs, &mut l);
        let off = l.tm_gmtoff / 60;
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z {}{:02}:{:02}",
            u.tm_year + 1900, u.tm_mon + 1, u.tm_mday, u.tm_hour, u.tm_min, u.tm_sec,
            if off < 0 { '-' } else { '+' }, off.abs() / 60, off.abs() % 60
        )
    }
}

fn log(msg: &str) {
    println!("[{}] {}", ts(), msg);
}

fn idle_secs() -> f64 {
    CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::CombinedSessionState,
        CGEventType(u32::MAX), // kCGAnyInputEventType
    )
}

/// Returns (pid, assertion name) for every process holding PreventUserIdleDisplaySleep.
unsafe fn display_sleep_holders() -> Result<Vec<(i64, String)>, i32> {
    let mut out: *mut c_void = std::ptr::null_mut();
    let rc = unsafe { IOPMCopyAssertionsByProcess(&mut out) };
    if rc != 0 || out.is_null() {
        return Err(rc);
    }
    // CFDictionary/CFArray are toll-free bridged to NSDictionary/NSArray.
    let dict = out as *mut AnyObject;
    let mut holders = Vec::new();
    unsafe {
        let keys: *mut AnyObject = msg_send![dict, allKeys];
        let n: usize = msg_send![keys, count];
        for i in 0..n {
            let k: *mut AnyObject = msg_send![keys, objectAtIndex: i];
            let pid: i64 = msg_send![k, longLongValue];
            let arr: *mut AnyObject = msg_send![dict, objectForKey: k];
            let m: usize = msg_send![arr, count];
            for j in 0..m {
                let a: *mut AnyObject = msg_send![arr, objectAtIndex: j];
                let ty: *mut NSString = msg_send![a, objectForKey: &*NSString::from_str("AssertType")];
                if !ty.is_null() && (*ty).to_string() == "PreventUserIdleDisplaySleep" {
                    let nm: *mut NSString =
                        msg_send![a, objectForKey: &*NSString::from_str("AssertName")];
                    let name = if nm.is_null() { "?".into() } else { (*nm).to_string() };
                    holders.push((pid, name));
                }
            }
        }
        let _: () = msg_send![dict, release];
    }
    Ok(holders)
}

fn proc_name(pid: i64) -> String {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr() as *mut c_void, 256) };
    if n > 0 { String::from_utf8_lossy(&buf[..n as usize]).into_owned() } else { "?".into() }
}

fn minute_check() {
    let s = idle_secs();
    log(&format!(
        "idle: {:.0}s since last input -> {}",
        s,
        if s >= IDLE_THRESHOLD_SECS { "idle" } else { "active" }
    ));
    match unsafe { display_sleep_holders() } {
        Ok(h) if h.is_empty() => log("assertion PreventUserIdleDisplaySleep: none"),
        Ok(h) => {
            let d: Vec<String> =
                h.iter().map(|(p, n)| format!("{}[{}] \"{}\"", proc_name(*p), p, n)).collect();
            log(&format!("assertion PreventUserIdleDisplaySleep: HELD by {}", d.join(", ")));
        }
        Err(rc) => log(&format!("assertion check failed: IOReturn {rc:#x}")),
    }
}

fn main() {
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        let nc = ws.notificationCenter();

        if let Some(app) = ws.frontmostApplication() {
            let name = app.localizedName().map(|n| n.to_string()).unwrap_or_default();
            let bid = app.bundleIdentifier().map(|n| n.to_string()).unwrap_or_default();
            *LAST_BUNDLE.lock().unwrap() = bid.clone();
            log(&format!("front app (initial): {name} ({bid})"));
        }

        let on_activate = RcBlock::new(|n: NonNull<NSNotification>| {
            let Some(info) = n.as_ref().userInfo() else { return };
            let Some(obj) = info.objectForKey(NSWorkspaceApplicationKey) else { return };
            let Ok(app) = obj.downcast::<NSRunningApplication>() else { return };
            let name = app.localizedName().map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            let bid = app.bundleIdentifier().map(|n| n.to_string()).unwrap_or_else(|| "?".into());
            let mut last = LAST_BUNDLE.lock().unwrap();
            if *last != bid {
                log(&format!("front app: {name} ({bid})"));
                *last = bid;
            }
        });
        let _o1 = nc.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification), None, None, &on_activate);

        macro_rules! power_obs {
            ($note:expr, $label:expr, $wake:expr) => {{
                let b = RcBlock::new(move |_: NonNull<NSNotification>| {
                    if $wake {
                        let mut ev = CYCLE_EVENTS.lock().unwrap();
                        ev.push($label);
                        log(&format!("{}", $label));
                        if $label == "didWake" {
                            log(&format!("sleep-cycle events since last wake: {:?}", *ev));
                            ev.clear();
                        }
                    } else {
                        CYCLE_EVENTS.lock().unwrap().push($label);
                        log(&format!("{}", $label));
                    }
                });
                nc.addObserverForName_object_queue_usingBlock(Some($note), None, None, &b)
            }};
        }
        let _o2 = power_obs!(NSWorkspaceWillSleepNotification, "willSleep", false);
        let _o3 = power_obs!(NSWorkspaceScreensDidSleepNotification, "screensDidSleep", false);
        let _o4 = power_obs!(NSWorkspaceScreensDidWakeNotification, "screensDidWake", true);
        let _o5 = power_obs!(NSWorkspaceDidWakeNotification, "didWake", true);

        let on_timer = RcBlock::new(|_: NonNull<NSTimer>| minute_check());
        let _timer = NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            CHECK_INTERVAL_SECS, true, &on_timer);

        minute_check();
        NSRunLoop::currentRunLoop().run();
    }
}

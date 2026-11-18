//! OS signals → `SystemEvent`s. Ported from the Milestone 0 spike.
//!
//! * Front app: `NSWorkspaceDidActivateApplicationNotification` (event-driven).
//! * Sleep / wake / display sleep / power-off: `NSWorkspace` notifications.
//! * Lock / unlock: `com.apple.screenIsLocked` / `com.apple.screenIsUnlocked`
//!   distributed notifications.
//! * Once a minute (run-loop `NSTimer`): seconds since last input
//!   (`CGEventSourceSecondsSinceLastEventType`) and processes holding a
//!   `PreventUserIdleDisplaySleep` assertion (`IOPMCopyAssertionsByProcess`),
//!   each marked as held by the frontmost app or not: the holder's pid, or the
//!   pid the assertion was created on behalf of (`AssertionOnBehalfOfPID`),
//!   equals `frontmostApplication.processIdentifier`. Both are public APIs
//!   with no permission. A child of the front app (e.g. `caffeinate` run in a
//!   terminal) is a different pid and does not count.
//!
//! Every callback body runs inside its own autorelease pool, so autoreleased
//! Objective-C objects are freed at the end of each callback instead of
//! accumulating (the likely cause of the spike's slow RSS growth, which ran
//! a bare `NSRunLoop` that never drained a pool).

use std::ffi::c_void;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
    NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceWillPowerOffNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationName, NSObjectProtocol,
    NSOperationQueue, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer,
};
use wrapped_core::{DisplayAssertion, SystemEvent};

use super::deliver;

type MakeEvent = fn() -> SystemEvent;

/// Idle check interval.
pub const TICK_SECS: f64 = 60.0;
/// Let the OS shift each tick by up to this much to batch wakeups.
const TICK_TOLERANCE_SECS: f64 = 5.0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Public IOKit power-management API: on success (0) returns a +1
    /// CFDictionary of pid (CFNumber) -> CFArray of assertion dictionaries.
    /// No entitlement or permission needed.
    fn IOPMCopyAssertionsByProcess(out: *mut *mut c_void) -> i32;
}

/// Keeps observers and the timer alive for the life of the app.
pub struct Sources {
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    _timer: Retained<NSTimer>,
}

fn front_app_event(app: &NSRunningApplication) -> SystemEvent {
    let bundle_id = app
        .bundleIdentifier()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let name = app
        .localizedName()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    SystemEvent::FrontApp { bundle_id, name }
}

fn idle_secs() -> f64 {
    CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::CombinedSessionState,
        CGEventType(u32::MAX), // kCGAnyInputEventType
    )
}

fn proc_name(pid: i64) -> String {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n > 0 {
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    } else {
        "?".into()
    }
}

/// Pid of the frontmost application, if any.
fn front_pid() -> Option<i64> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| i64::from(app.processIdentifier()))
        .filter(|&pid| pid > 0)
}

/// Every `PreventUserIdleDisplaySleep` assertion, as `"name[pid] assertion
/// name"`, marked `front_app` when held by `front` or created on its behalf.
/// Errors count as "none".
fn display_assertions(front: Option<i64>) -> Vec<DisplayAssertion> {
    let mut out: *mut c_void = std::ptr::null_mut();
    if unsafe { IOPMCopyAssertionsByProcess(&mut out) } != 0 || out.is_null() {
        return Vec::new();
    }
    // CFDictionary/CFArray/CFString are toll-free bridged to their NS types.
    // Take ownership of the +1 reference so it is released on every path.
    let Some(dict) = (unsafe { Retained::from_raw(out.cast::<AnyObject>()) }) else {
        return Vec::new();
    };
    let type_key = NSString::from_str("AssertType");
    let name_key = NSString::from_str("AssertName");
    // kIOPMAssertionOnBehalfOfPID (IOPMLib.h): set when a daemon creates the
    // assertion for a client app (pmset shows it as "Created for PID").
    let behalf_key = NSString::from_str("AssertionOnBehalfOfPID");
    let mut holders = Vec::new();
    unsafe {
        let keys: *mut AnyObject = msg_send![&*dict, allKeys];
        let n: usize = msg_send![keys, count];
        for i in 0..n {
            let k: *mut AnyObject = msg_send![keys, objectAtIndex: i];
            let pid: i64 = msg_send![k, longLongValue];
            let arr: *mut AnyObject = msg_send![&*dict, objectForKey: k];
            if arr.is_null() {
                continue;
            }
            let m: usize = msg_send![arr, count];
            for j in 0..m {
                let a: *mut AnyObject = msg_send![arr, objectAtIndex: j];
                let ty: *mut NSString = msg_send![a, objectForKey: &*type_key];
                if !ty.is_null() && (*ty).to_string() == "PreventUserIdleDisplaySleep" {
                    let nm: *mut NSString = msg_send![a, objectForKey: &*name_key];
                    let name = if nm.is_null() {
                        "?".to_string()
                    } else {
                        (*nm).to_string()
                    };
                    let behalf: *mut AnyObject = msg_send![a, objectForKey: &*behalf_key];
                    let behalf: Option<i64> =
                        (!behalf.is_null()).then(|| msg_send![behalf, longLongValue]);
                    let front_app = front.is_some_and(|f| f == pid || behalf == Some(f));
                    holders.push(DisplayAssertion::new(
                        format!("{}[{pid}] {name}", proc_name(pid)),
                        front_app,
                    ));
                }
            }
        }
    }
    holders
}

fn tick() {
    deliver(SystemEvent::IdleTick {
        secs_since_input: idle_secs(),
        display_assertions: display_assertions(front_pid()),
    });
}

/// Current front app plus an immediate idle check.
pub fn send_initial_state() {
    autoreleasepool(|_| {
        if let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() {
            deliver(front_app_event(&app));
        }
        tick();
    });
}

pub fn install() -> Sources {
    let main_queue = NSOperationQueue::mainQueue();
    let ws_center = NSWorkspace::sharedWorkspace().notificationCenter();
    let mut observers = Vec::new();

    let on_activate = RcBlock::new(|n: NonNull<NSNotification>| {
        autoreleasepool(|_| {
            let n = unsafe { n.as_ref() };
            let Some(info) = n.userInfo() else { return };
            let Some(obj) = info.objectForKey(unsafe { NSWorkspaceApplicationKey }) else {
                return;
            };
            if let Ok(app) = obj.downcast::<NSRunningApplication>() {
                deliver(front_app_event(&app));
            }
        })
    });
    observers.push(unsafe {
        ws_center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            Some(&main_queue),
            &on_activate,
        )
    });

    let simple: [(&NSNotificationName, MakeEvent); 5] = unsafe {
        [
            (NSWorkspaceWillSleepNotification, || SystemEvent::WillSleep),
            (NSWorkspaceDidWakeNotification, || SystemEvent::DidWake),
            (NSWorkspaceScreensDidSleepNotification, || {
                SystemEvent::ScreensSlept
            }),
            (NSWorkspaceScreensDidWakeNotification, || {
                SystemEvent::ScreensWoke
            }),
            (NSWorkspaceWillPowerOffNotification, || {
                SystemEvent::WillPowerOff
            }),
        ]
    };
    for (name, make) in simple {
        let block =
            RcBlock::new(move |_: NonNull<NSNotification>| autoreleasepool(|_| deliver(make())));
        observers.push(unsafe {
            ws_center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&main_queue),
                &block,
            )
        });
    }

    let dist = NSDistributedNotificationCenter::defaultCenter();
    let lock_events: [(&str, MakeEvent); 2] = [
        ("com.apple.screenIsLocked", || SystemEvent::ScreenLocked),
        ("com.apple.screenIsUnlocked", || SystemEvent::ScreenUnlocked),
    ];
    for (name, make) in lock_events {
        let name = NSString::from_str(name);
        let block =
            RcBlock::new(move |_: NonNull<NSNotification>| autoreleasepool(|_| deliver(make())));
        observers.push(unsafe {
            dist.addObserverForName_object_queue_usingBlock(
                Some(&name),
                None,
                Some(&main_queue),
                &block,
            )
        });
    }

    let on_timer = RcBlock::new(|_: NonNull<NSTimer>| autoreleasepool(|_| tick()));
    let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(TICK_SECS, true, &on_timer) };
    timer.setTolerance(TICK_TOLERANCE_SECS);
    // Common modes: keep ticking while the tray menu is open.
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };

    Sources {
        _observers: observers,
        _timer: timer,
    }
}

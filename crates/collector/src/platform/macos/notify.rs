//! Recap notifications (DESIGN.md §11) through `UNUserNotificationCenter`,
//! called with `msg_send!` (the UserNotifications framework is linked here;
//! no binding crate). What to announce and when is decided in
//! `wrapped_core::recap::notify`; this file only posts, asks for permission
//! and handles the click.
//!
//! * Checked on the minute tick, at most once an hour, and on the first tick
//!   after a wake (no timer or thread of its own).
//! * Permission is asked only when a recap toggle is turned on. Its state is
//!   kept in `internal.notify_permission` for the dashboard's Settings.
//! * The notification's identifier is the recap id; clicking it leaves the
//!   `open-recap` flag next to the database and opens the dashboard, which
//!   opens that recap.
//! * The notification center only exists for a process inside an app bundle;
//!   a bare binary (`cargo run`) logs and posts nothing.

use std::ffi::CStr;
use std::path::Path;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{AnyThread, define_class, msg_send};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSObject, NSObjectProtocol, NSString};
use wrapped_core::recap::notify::{self as rn, Notice};
use wrapped_core::recap::{Period, PeriodKind, build, list};
use wrapped_core::stats::Stats;
use wrapped_core::{Clock, Engine, SystemClock, Timestamp, collectors};

use super::{log, with_state};

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// How often the hourly check runs (wall clock).
const CHECK_EVERY_MS: i64 = 3_600_000;

/// `UNAuthorizationOptionSound | UNAuthorizationOptionAlert`.
const AUTH_OPTIONS: usize = 2 | 4;
/// `UNNotificationPresentationOptionList | ...Banner | ...Sound`.
const PRESENT_OPTIONS: usize = (1 << 3) | (1 << 4) | (1 << 1);

/// Per-process notification state (on the collector's main thread).
#[derive(Default)]
pub struct NotifyState {
    pub(super) last_check_ms: Option<i64>,
    /// Toggles at the last look (week, month, year), to see one turned on.
    prev: [bool; 3],
    started: bool,
}

fn class(name: &CStr) -> Option<&'static AnyClass> {
    AnyClass::get(name)
}

/// The notification center, if this process may use one (inside an .app).
fn center() -> Option<Retained<AnyObject>> {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier()?;
    if !bundle.bundlePath().to_string().ends_with(".app") {
        return None;
    }
    let cls = class(c"UNUserNotificationCenter")?;
    unsafe { msg_send![cls, currentNotificationCenter] }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; no Drop, no ivars.
    // The center calls these on a queue of its own: everything that touches
    // the collector hops to the main queue.
    #[unsafe(super(NSObject))]
    #[name = "WrappedNotificationDelegate"]
    pub struct NotifyDelegate;

    unsafe impl NSObjectProtocol for NotifyDelegate {}

    impl NotifyDelegate {
        /// A click on a notification (or its default action).
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &AnyObject,
            response: &AnyObject,
            handler: &block2::Block<dyn Fn()>,
        ) {
            let id = autoreleasepool(|_| unsafe {
                let n: Option<Retained<AnyObject>> = msg_send![response, notification];
                let r: Option<Retained<AnyObject>> = n.and_then(|n| msg_send![&*n, request]);
                let s: Option<Retained<NSString>> = r.and_then(|r| msg_send![&*r, identifier]);
                s.map(|s| s.to_string())
            });
            if let Some(id) = id.filter(|i| Period::parse_id(i).is_some()) {
                DispatchQueue::main().exec_async(move || open_recap(&id));
            }
            handler.call(());
        }

        /// Show it even if Wrapped counts as the active app.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &AnyObject,
            _notification: &AnyObject,
            handler: &block2::Block<dyn Fn(usize)>,
        ) {
            handler.call((PRESENT_OPTIONS,));
        }
    }
);

impl NotifyDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// The center keeps only a weak reference to its delegate.
    static DELEGATE: std::cell::OnceCell<Retained<NotifyDelegate>> = const { std::cell::OnceCell::new() };
}

/// Install the click handler (before the app runs, so a click that launches
/// Wrapped is delivered too) and read the permission state.
pub fn install() {
    autoreleasepool(|_| {
        let Some(c) = center() else {
            log("notifications: not inside an app bundle; recap notifications are off");
            store_permission("unavailable");
            return;
        };
        let d = DELEGATE.with(|cell| cell.get_or_init(NotifyDelegate::new).clone());
        unsafe {
            let _: () = msg_send![&*c, setDelegate: &*d];
        }
        refresh_permission();
    });
}

fn store_permission(state: &str) {
    with_state(|st| {
        if let Err(e) = st
            .engine
            .store()
            .set_internal_value(rn::PERMISSION_KEY, state)
        {
            log(&format!(
                "notifications: cannot store permission state: {e}"
            ));
        }
    });
}

/// `UNAuthorizationStatus` as a word.
fn status_name(s: isize) -> &'static str {
    match s {
        0 => "not_determined",
        1 => "denied",
        _ => "granted", // authorized, provisional, ephemeral
    }
}

/// Read the permission state (asynchronously) into the database.
fn refresh_permission() {
    let Some(c) = center() else { return };
    let block = RcBlock::new(|settings: *mut AnyObject| {
        let status: isize = if settings.is_null() {
            0
        } else {
            unsafe { msg_send![&*settings, authorizationStatus] }
        };
        let name = status_name(status);
        DispatchQueue::main().exec_async(move || store_permission(name));
    });
    unsafe {
        let _: () = msg_send![&*c, getNotificationSettingsWithCompletionHandler: &*block];
    }
}

/// Ask for permission (shows the system prompt only the first time), then
/// run a check.
fn request_permission() {
    let Some(c) = center() else { return };
    let block = RcBlock::new(|granted: Bool, _err: *mut AnyObject| {
        let granted = granted.as_bool();
        DispatchQueue::main().exec_async(move || {
            log(&format!(
                "notifications: permission {}",
                if granted { "granted" } else { "not granted" }
            ));
            store_permission(if granted { "granted" } else { "denied" });
            with_state(|st| st.notify.last_check_ms = None);
            check_now();
        });
    });
    unsafe {
        let _: () = msg_send![&*c, requestAuthorizationWithOptions: AUTH_OPTIONS, completionHandler: &*block];
    }
}

fn toggles(e: &Engine) -> [bool; 3] {
    PeriodKind::ALL.map(|k| rn::enabled(e.settings(), k))
}

/// After settings may have changed (a request, each tick): a toggle turned
/// on stores its baseline and asks for permission.
pub fn settings_changed() {
    let turned_on = with_state(|st| {
        let now = toggles(&st.engine);
        let prev = std::mem::replace(&mut st.notify.prev, now);
        if !st.notify.started {
            // The first look after start is not a change.
            st.notify.started = true;
            return false;
        }
        let on = now.iter().zip(prev).any(|(n, p)| *n && !p);
        if on
            && let Err(e) =
                rn::ensure_baselines(st.engine.store(), st.engine.settings(), SystemClock.now())
        {
            log(&format!("notifications: cannot store baseline: {e}"));
        }
        on
    })
    .unwrap_or(false);
    if turned_on {
        request_permission();
    }
}

/// Minute tick: the hourly check.
pub fn tick(now: Timestamp) {
    let due = with_state(|st| {
        rn::any_enabled(st.engine.settings())
            && st
                .notify
                .last_check_ms
                .is_none_or(|t| now.utc_ms - t >= CHECK_EVERY_MS)
    })
    .unwrap_or(false);
    if due {
        check_now();
    }
}

fn check_now() {
    let notices = with_state(|st| {
        let now = SystemClock.now();
        st.notify.last_check_ms = Some(now.utc_ms);
        let all = collectors::builtin();
        match rn::check(st.engine.store(), st.engine.settings(), &all, now, app_name) {
            Ok(n) => n,
            Err(e) => {
                log(&format!("notifications: check failed: {e}"));
                Vec::new()
            }
        }
    })
    .unwrap_or_default();
    for n in &notices {
        post(n);
    }
}

/// Post one notification; its identifier is the recap id.
fn post(n: &Notice) -> bool {
    autoreleasepool(|_| {
        let (Some(c), Some(content_cls), Some(req_cls), Some(sound_cls)) = (
            center(),
            class(c"UNMutableNotificationContent"),
            class(c"UNNotificationRequest"),
            class(c"UNNotificationSound"),
        ) else {
            return false;
        };
        unsafe {
            let content: Retained<AnyObject> = msg_send![content_cls, new];
            let _: () = msg_send![&*content, setTitle: &*NSString::from_str(&n.title)];
            let _: () = msg_send![&*content, setBody: &*NSString::from_str(&n.body)];
            let sound: Option<Retained<AnyObject>> = msg_send![sound_cls, defaultSound];
            if let Some(s) = sound {
                let _: () = msg_send![&*content, setSound: &*s];
            }
            let id = NSString::from_str(&n.period.id());
            let none: Option<&AnyObject> = None;
            let req: Option<Retained<AnyObject>> = msg_send![
                req_cls,
                requestWithIdentifier: &*id,
                content: &*content,
                trigger: none
            ];
            let Some(req) = req else { return false };
            let done = RcBlock::new(|err: *mut AnyObject| {
                if !err.is_null() {
                    DispatchQueue::main().exec_async(|| log("notifications: posting failed"));
                }
            });
            let _: () =
                msg_send![&*c, addNotificationRequest: &*req, withCompletionHandler: &*done];
        }
        log(&format!(
            "notifications: posted {} ({})",
            n.period.id(),
            n.body
        ));
        true
    })
}

/// A click: open the dashboard on that recap.
fn open_recap(id: &str) {
    let Some(db) = with_state(|st| st.db.clone()) else {
        return;
    };
    write_open_recap_flag(&db, id);
    if let Err(e) = super::open_dashboard(&db) {
        log(&format!("notifications: cannot open the dashboard: {e}"));
    }
}

fn write_open_recap_flag(db: &Path, id: &str) {
    let flag = wrapped_core::ctl::open_recap_flag(db);
    if let Some(dir) = flag.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&flag, id);
}

/// `ctl recap_notify_test {"kind": "week"}`: post the notification of the
/// newest available recap of that kind now (for testing; marks nothing).
pub fn test(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let kind = match args.get("kind").and_then(|v| v.as_str()) {
        None => PeriodKind::Week,
        Some(k) => PeriodKind::parse(k).ok_or_else(|| format!("unknown kind `{k}`"))?,
    };
    if center().is_none() {
        return Err("notifications need Wrapped.app (this collector is a bare binary)".into());
    }
    let notice = with_state(|st| -> Result<Notice, String> {
        let store = st.engine.store();
        let all = collectors::builtin();
        let stats = Stats::new(store, &all, st.engine.settings());
        let today = SystemClock.now().local_date();
        let entries = list(&stats, today).map_err(|e| e.to_string())?;
        let e = entries
            .iter()
            .find(|e| e.period.kind == kind)
            .ok_or_else(|| format!("no {} recap yet", kind.name()))?;
        let r = build(&stats, e.period, today)
            .map_err(|e| e.to_string())?
            .ok_or("recap not started")?;
        Ok(rn::notice(&r, &app_name))
    })
    .ok_or("collector is shutting down")??;
    let permission = with_state(|st| st.engine.store().internal_value(rn::PERMISSION_KEY))
        .and_then(Result::ok)
        .flatten()
        .unwrap_or_default();
    if permission != "granted" {
        request_permission();
    }
    post(&notice);
    Ok(
        serde_json::json!({ "posted": notice.period.id(), "body": notice.body, "permission": permission }),
    )
}

/// Display name of an app id (as the dashboard shows it); the id itself
/// when no installed app has it.
fn app_name(id: &str) -> String {
    autoreleasepool(|_| {
        let ws = NSWorkspace::sharedWorkspace();
        let Some(url) = ws.URLForApplicationWithBundleIdentifier(&NSString::from_str(id)) else {
            return id.to_string();
        };
        if let Some(bundle) = NSBundle::bundleWithURL(&url) {
            for key in ["CFBundleDisplayName", "CFBundleName"] {
                let v: Option<Retained<AnyObject>> =
                    bundle.objectForInfoDictionaryKey(&NSString::from_str(key));
                if let Some(s) = v.and_then(|v| v.downcast::<NSString>().ok()) {
                    let s = s.to_string();
                    if !s.trim().is_empty() {
                        return s;
                    }
                }
            }
        }
        id.to_string()
    })
}

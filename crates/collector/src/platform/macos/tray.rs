//! Native menu-bar icon (NSStatusItem + NSMenu; no web view).
//!
//! Reopening the app while it runs opens the dashboard too.
//!
//! Menu: status line, Open Dashboard…, Settings… (Finish Setup… until the
//! first-launch checklist is done), Pause for 1 hour / until tomorrow /
//! indefinitely, Resume, Open data folder, Quit. One main-thread-only Objective-C class is
//! the app delegate (to flush on termination), the menu delegate (to refresh
//! the status line when the menu opens) and the target of every menu item.

use std::cell::{OnceCell, RefCell};
use std::path::PathBuf;

use chrono::{Local, TimeZone};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar,
    NSStatusItem, NSWorkspace,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString, NSURL};
use wrapped_core::{Clock, Pause, SystemClock};

use super::{log, with_state};

/// `NSSquareStatusItemLength`
const SQUARE_LENGTH: f64 = -2.0;

pub struct Ivars {
    data_dir: PathBuf,
    db: PathBuf,
    status_item: OnceCell<Retained<NSStatusItem>>,
    status_line: OnceCell<Retained<NSMenuItem>>,
    resume_item: OnceCell<Retained<NSMenuItem>>,
    settings_item: OnceCell<Retained<NSMenuItem>>,
    shown_paused: std::cell::Cell<Option<bool>>,
    shown_setup: std::cell::Cell<Option<bool>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Delegate has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WrappedCollectorDelegate"]
    #[ivars = Ivars]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        /// Quit menu, SIGTERM/SIGINT (via terminate:) and logout all land here.
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            autoreleasepool(|_| super::shutdown());
        }

        /// Opening Wrapped.app while it is already running (Spotlight, Finder,
        /// `open`) opens the dashboard. A fresh launch gets no such call, so a
        /// launch at login or the relaunch after an install stays silent.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            autoreleasepool(|_| {
                log("app reopened: opening the dashboard");
                if let Err(e) = super::open_dashboard(&self.ivars().db) {
                    log(&format!("reopen: Open Dashboard: {e}"));
                }
            });
            false // nothing else to do (no windows to bring back)
        }
    }

    unsafe impl NSMenuDelegate for Delegate {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            autoreleasepool(|_| self.refresh());
        }
    }

    impl Delegate {
        #[unsafe(method(pauseHour:))]
        fn pause_hour(&self, _sender: Option<&AnyObject>) {
            self.set_pause(Pause::for_one_hour, "Pause for 1 hour");
        }

        #[unsafe(method(pauseTomorrow:))]
        fn pause_tomorrow(&self, _sender: Option<&AnyObject>) {
            self.set_pause(Pause::until_tomorrow, "Pause until tomorrow");
        }

        #[unsafe(method(pauseIndefinitely:))]
        fn pause_indefinitely(&self, _sender: Option<&AnyObject>) {
            self.set_pause(|_| Pause::Indefinite, "Pause indefinitely");
        }

        #[unsafe(method(resume:))]
        fn resume(&self, _sender: Option<&AnyObject>) {
            self.set_pause(|_| Pause::Off, "Resume");
        }

        #[unsafe(method(openDashboard:))]
        fn open_dashboard(&self, _sender: Option<&AnyObject>) {
            if let Err(e) = super::open_dashboard(&self.ivars().db) {
                log(&format!("tray: Open Dashboard: {e}"));
            }
        }

        /// "Settings…": the dashboard with the Settings window open (or, until
        /// the first-launch checklist is done, the checklist). The dashboard
        /// finds `open-settings` next to the database when it starts or gains focus.
        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            let in_setup = with_state(|st| st.engine.is_setup_mode()).unwrap_or(false);
            let db = &self.ivars().db;
            if !in_setup {
                let flag = wrapped_core::ctl::open_settings_flag(db);
                if let Some(dir) = flag.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&flag, "1");
            }
            if let Err(e) = super::open_dashboard(db) {
                log(&format!("tray: Settings: {e}"));
            }
        }

        #[unsafe(method(openDataFolder:))]
        fn open_data_folder(&self, _sender: Option<&AnyObject>) {
            autoreleasepool(|_| {
                let path = NSString::from_str(&self.ivars().data_dir.to_string_lossy());
                let url = NSURL::fileURLWithPath(&path);
                NSWorkspace::sharedWorkspace().openURL(&url);
            });
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

thread_local! {
    static TRAY: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

impl Delegate {
    pub fn new(mtm: MainThreadMarker, data_dir: PathBuf, db: PathBuf) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            data_dir,
            db,
            status_item: OnceCell::new(),
            status_line: OnceCell::new(),
            resume_item: OnceCell::new(),
            settings_item: OnceCell::new(),
            shown_paused: std::cell::Cell::new(None),
            shown_setup: std::cell::Cell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn set_pause(&self, make: impl FnOnce(wrapped_core::Timestamp) -> Pause, label: &str) {
        autoreleasepool(|_| {
            with_state(|st| {
                let pause = make(SystemClock.now());
                st.measured(label, |e| e.set_pause(pause).map(|()| None));
                if st.debug {
                    log(&format!("tray: {label}"));
                }
            });
            self.refresh();
        });
    }

    /// Update the status line, Resume availability and icon from the engine.
    fn refresh(&self) {
        let Some(pause) = with_state(|st| {
            let p = st.engine.settings().pause;
            (st.engine.is_paused(), p, st.engine.is_setup_mode())
        }) else {
            return;
        };
        let (paused, p, setup) = pause;
        let iv = self.ivars();
        if let Some(line) = iv.status_line.get() {
            let text = if setup {
                "Wrapped: finish setup to start recording".to_string()
            } else {
                status_text(paused, p)
            };
            line.setTitle(&NSString::from_str(&text));
        }
        if iv.shown_setup.get() != Some(setup)
            && let Some(it) = iv.settings_item.get()
        {
            iv.shown_setup.set(Some(setup));
            it.setTitle(&NSString::from_str(if setup {
                "Finish Setup…"
            } else {
                "Settings…"
            }));
        }
        if let Some(r) = iv.resume_item.get() {
            r.setEnabled(paused);
        }
        if iv.shown_paused.get() != Some(paused) {
            iv.shown_paused.set(Some(paused));
            if let Some(item) = iv.status_item.get() {
                set_icon(item, paused, self.mtm());
            }
        }
    }
}

fn status_text(paused: bool, p: Pause) -> String {
    if !paused {
        return "Wrapped: tracking".into();
    }
    match p {
        Pause::Until(ms) => match Local.timestamp_millis_opt(ms).single() {
            Some(t) => {
                let today = Local::now().date_naive();
                let day = if t.date_naive() == today {
                    String::new()
                } else if Some(t.date_naive()) == today.succ_opt() {
                    "tomorrow ".into()
                } else {
                    t.format("%Y-%m-%d ").to_string()
                };
                format!("Wrapped: paused until {day}{}", t.format("%H:%M"))
            }
            None => "Wrapped: paused".into(),
        },
        _ => "Wrapped: paused indefinitely".into(),
    }
}

fn set_icon(item: &NSStatusItem, paused: bool, mtm: MainThreadMarker) {
    let Some(button) = item.button(mtm) else {
        return;
    };
    let symbol = if paused {
        "pause.circle"
    } else {
        "chart.bar.fill"
    };
    let desc = NSString::from_str(if paused {
        "Wrapped (paused)"
    } else {
        "Wrapped"
    });
    match NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&desc),
    ) {
        Some(img) => {
            img.setTemplate(true);
            button.setImage(Some(&img));
        }
        None => button.setTitle(&NSString::from_str(if paused { "W‖" } else { "W" })),
    }
}

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
    target: &Delegate,
) -> Retained<NSMenuItem> {
    let it = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if action.is_some() {
        unsafe { it.setTarget(Some(target)) };
    }
    it
}

/// Create the status item and its menu.
pub fn install(delegate: &Retained<Delegate>, mtm: MainThreadMarker) {
    let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(SQUARE_LENGTH);
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));

    let status_line = item(mtm, "Wrapped: tracking", None, "", delegate);
    status_line.setEnabled(false);
    menu.addItem(&status_line);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(
        mtm,
        "Open Dashboard…",
        Some(sel!(openDashboard:)),
        "d",
        delegate,
    ));
    let settings_item = item(mtm, "Settings…", Some(sel!(openSettings:)), ",", delegate);
    menu.addItem(&settings_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(
        mtm,
        "Pause for 1 Hour",
        Some(sel!(pauseHour:)),
        "",
        delegate,
    ));
    menu.addItem(&item(
        mtm,
        "Pause Until Tomorrow",
        Some(sel!(pauseTomorrow:)),
        "",
        delegate,
    ));
    menu.addItem(&item(
        mtm,
        "Pause Indefinitely",
        Some(sel!(pauseIndefinitely:)),
        "",
        delegate,
    ));
    let resume = item(mtm, "Resume", Some(sel!(resume:)), "", delegate);
    menu.addItem(&resume);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(
        mtm,
        "Open Data Folder",
        Some(sel!(openDataFolder:)),
        "",
        delegate,
    ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(mtm, "Quit Wrapped", Some(sel!(quit:)), "q", delegate));
    status_item.setMenu(Some(&menu));

    let iv = delegate.ivars();
    let _ = iv.status_item.set(status_item);
    let _ = iv.status_line.set(status_line);
    let _ = iv.resume_item.set(resume);
    let _ = iv.settings_item.set(settings_item);
    TRAY.with(|t| *t.borrow_mut() = Some(delegate.clone()));
    delegate.refresh();
}

/// Refresh after a tick (a timed pause may have expired).
pub fn refresh() {
    let d = TRAY.with(|t| t.borrow().clone());
    if let Some(d) = d {
        d.refresh();
    }
}

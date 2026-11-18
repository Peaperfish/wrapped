//! macOS platform layer.
//!
//! Everything runs on the main thread inside `NSApplication`'s run loop: OS
//! notifications and the minute timer are delivered on the main queue, the
//! tray menu acts on the main thread, and signals are forwarded to the main
//! queue by a dedicated `sigwait` thread. The engine therefore lives in a
//! main-thread `thread_local` and is never shared across threads.
//!
//! Permissions: none. No Accessibility, Input Monitoring or Screen Recording,
//! no event taps or keyboard hooks. Idle time comes from
//! `CGEventSourceSecondsSinceLastEventType`, display assertions from
//! `IOPMCopyAssertionsByProcess`, everything else from notifications. Disk
//! and memory counters (Disk & Memory collector) come from IOKit, Mach VM
//! statistics and sysctl (see `counters.rs`).

mod bookmarks;
mod counters;
mod lock;
mod login;
mod scan;
mod signals;
mod sources;
mod tray;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use wrapped_core::store::FlushReport;
use wrapped_core::time::{MS_PER_DAY, Timestamp};
use wrapped_core::{Clock, Engine, Store, SystemClock, SystemEvent, collectors};

pub use counters::disk_memory;
pub use login::login;
pub use scan::{children_cpu_secs, hold_db_lock, scan_hooks, usage};

/// Gap between idle ticks (with no sleep in between, system awake) above which
/// a warning is printed even without debug logging. The timer interval is 60 s.
/// Ticks between `WillSleep` and `DidWake` come from dark wakes (maintenance
/// wakes, which send no wake notification) and are logged as such instead.
const TICK_GAP_WARN_SECS: f64 = 90.0;

/// `~/Library/Application Support/Wrapped`
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Wrapped"))
}

fn lock_path(db: &Path) -> PathBuf {
    db.with_extension("lock")
}

/// The dashboard nested in this app bundle.
pub const DASHBOARD_APP: &str = "Contents/Helpers/Wrapped Dashboard.app";

/// Open the dashboard on `db`, or bring its window to the front if it is
/// already running. Inside `Wrapped.app`, `open`s the nested
/// `Wrapped Dashboard.app` (Launch Services keeps it to one instance; the
/// arguments only apply to a fresh launch). A bare binary (`cargo run`)
/// starts the `wrapped-dashboard` binary next to it instead.
pub fn open_dashboard(db: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;
    let dir = exe.parent().unwrap_or(Path::new("."));
    // <bundle>/Contents/MacOS/wrapped-collector -> <bundle>
    let nested = dir
        .parent()
        .and_then(Path::parent)
        .map(|bundle| bundle.join(DASHBOARD_APP));
    let mut cmd = match nested {
        Some(app) if app.is_dir() => {
            let mut c = std::process::Command::new("/usr/bin/open");
            c.arg(app).arg("--args").arg("--db").arg(db);
            c
        }
        _ => {
            let bin = dir.join("wrapped-dashboard");
            if !bin.is_file() {
                return Err(format!(
                    "no dashboard found ({DASHBOARD_APP} in the app bundle, or {})",
                    bin.display()
                ));
            }
            let mut c = std::process::Command::new(bin);
            c.arg("--db").arg(db);
            c
        }
    };
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot start the dashboard: {e}"))?;
    // Reap it off the main thread (`open` exits at once; a bare dashboard
    // binary lives until its window closes).
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

pub fn running_pid(db: &Path) -> Option<i32> {
    lock::running_pid(&lock_path(db))
}

/// Ask a running collector to flush now (SIGUSR1).
pub fn request_flush(db: &Path) -> Option<i32> {
    let pid = running_pid(db)?;
    (unsafe { libc::kill(pid, libc::SIGUSR1) } == 0).then_some(pid)
}

/// Wake the running collector so it reads its request folder (SIGUSR2).
/// False when none is running.
pub fn wake_for_ctl(db: &Path) -> bool {
    running_pid(db).is_some_and(|pid| unsafe { libc::kill(pid, libc::SIGUSR2) } == 0)
}

/// Timestamped line on stderr.
pub(crate) fn log(msg: &str) {
    eprintln!("[{}] {msg}", SystemClock.now().to_rfc3339());
}

/// Bytes this process has written to disk, as counted by the kernel (the
/// figure Activity Monitor shows in Disk → Bytes Written).
pub(crate) fn process_disk_bytes_written() -> Option<u64> {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        )
    };
    (rc == 0).then_some(info.ri_diskio_byteswritten)
}

struct State {
    engine: Engine,
    debug: bool,
    db: PathBuf,
    last_tick: Option<SystemTime>,
    slept_since_tick: bool,
    /// From `WillSleep` until `DidWake`: a tick now is a dark-wake tick.
    asleep_since: Option<Timestamp>,
    last_disk_bytes: u64,
}

/// Local wall-clock "HH:MM:SS".
fn hms(t: Timestamp) -> String {
    let s = t.local_ms().rem_euclid(MS_PER_DAY) / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Run `f` with the collector state; `None` after shutdown.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

impl State {
    /// Run an engine operation; in debug mode, describe any flush it caused.
    fn measured(
        &mut self,
        what: &str,
        op: impl FnOnce(&mut Engine) -> wrapped_core::store::Result<Option<FlushReport>>,
    ) {
        match op(&mut self.engine) {
            Ok(Some(report)) if self.debug => self.log_flush(what, &report),
            Ok(_) => {}
            Err(e) => log(&format!("error during {what}: {e}")),
        }
        for r in self.engine.take_scan_reports() {
            log(&format!(
                "scan {} {} in {:.1}s: {:?}",
                r.collector,
                if r.completed {
                    "completed"
                } else {
                    "stopped early (progress saved)"
                },
                (r.finished.utc_ms - r.started.utc_ms) as f64 / 1000.0,
                r.counters
            ));
        }
    }

    fn log_flush(&mut self, what: &str, r: &FlushReport) {
        let wal = std::fs::metadata(self.db.with_extension("db-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
        let disk = process_disk_bytes_written().unwrap_or(0);
        let delta = disk.saturating_sub(self.last_disk_bytes);
        self.last_disk_bytes = disk;
        log(&format!(
            "flush ({what}): {} sessions written, {} deleted, {} samples, {} day(s) recomputed, \
             {} metric rows changed | kernel-counted disk bytes written by this process: \
             +{delta} since last flush log, {disk} total | WAL file size {wal} bytes",
            r.sessions_written,
            r.sessions_deleted,
            r.samples_written,
            r.days_recomputed,
            r.metric_rows_changed
        ));
    }
}

/// Feed one OS event to the engine.
pub(crate) fn deliver(ev: SystemEvent) {
    let is_tick = matches!(ev, SystemEvent::IdleTick { .. });
    with_state(|st| {
        match ev {
            SystemEvent::WillSleep => {
                st.slept_since_tick = true;
                st.asleep_since = Some(SystemClock.now());
            }
            SystemEvent::DidWake => {
                st.slept_since_tick = true;
                st.asleep_since = None;
            }
            _ => {}
        }
        if is_tick {
            let now = SystemTime::now();
            let gap = st.last_tick.map(|prev| {
                now.duration_since(prev)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(f64::NAN)
            });
            if let Some(since) = st.asleep_since {
                // Never a warning: the timer runs during dark wakes.
                let gap = gap.map_or(String::new(), |g| format!(", {g:.1}s since previous tick"));
                log(&format!(
                    "tick during dark wake (asleep since {}){gap}",
                    hms(since)
                ));
            } else if let Some(gap) = gap {
                let note = if st.slept_since_tick {
                    " (sleep/wake in between)"
                } else {
                    ""
                };
                if st.debug {
                    log(&format!("tick gap {gap:.1}s since previous tick{note}"));
                }
                if !st.slept_since_tick && gap > TICK_GAP_WARN_SECS {
                    log(&format!(
                        "WARNING: idle tick arrived {gap:.1}s after the previous one with no \
                         sleep in between (expected ~60 s; App Nap / timer coalescing?)"
                    ));
                }
            }
            st.last_tick = Some(now);
            st.slept_since_tick = false;
        }
        let name = ev.name();
        st.measured(name, |e| e.handle(ev));
        if st.asleep_since.is_some() && !st.engine.is_asleep() {
            // Input after WillSleep with no DidWake: the wake was missed.
            st.asleep_since = None;
        }
    });
    if is_tick {
        process_ctl();
    }
}

/// Answer every request waiting in the request folder (SIGUSR2, and at each
/// minute tick as a safety net). Each is handled to completion on the main
/// thread before the next.
pub(crate) fn process_ctl() {
    with_state(|st| {
        for (id, req) in wrapped_core::ctl::take_pending(&st.db) {
            let reply = st.engine.handle_request(&req);
            if st.debug || matches!(reply, wrapped_core::ctl::Reply::Err { .. }) {
                log(&format!(
                    "request {}: {}",
                    req.cmd,
                    match &reply {
                        wrapped_core::ctl::Reply::Ok(_) => "ok".to_string(),
                        wrapped_core::ctl::Reply::Err { code, error } => format!("{code}: {error}"),
                    }
                ));
            }
            wrapped_core::ctl::reply(&st.db, &id, &reply);
            // Anything that wrote records (delete, pause) is visible at once;
            // a status poll writes nothing.
            if !matches!(req.cmd.as_str(), "ping" | "status") {
                st.measured("request", |e| e.flush().map(Some));
            }
        }
    });
    tray::refresh();
}

/// A scan thread wants a flush, or has finished (via the main queue).
fn poll_scans() {
    with_state(|st| st.measured("scan", |e| e.poll_scans()));
}

/// Flush now (SIGUSR1).
pub(crate) fn flush_now() {
    with_state(|st| st.measured("SIGUSR1", |e| e.flush().map(Some)));
}

/// Stop collectors and write everything. Runs once, from
/// `applicationWillTerminate:` (Quit menu, SIGTERM/SIGINT, logout).
pub(crate) fn shutdown() {
    let st = STATE.with(|s| s.borrow_mut().take());
    if let Some(mut st) = st {
        match st.engine.shutdown() {
            Ok(r) => {
                if st.debug {
                    st.log_flush("shutdown", &r);
                }
                log("collector stopped; all data written");
            }
            Err(e) => log(&format!("error during shutdown flush: {e}")),
        }
    }
}

pub fn run_collector(db: &Path, debug: bool) -> i32 {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("wrapped-collector: must run on the main thread");
        return 1;
    };
    let _lock = match lock::acquire(&lock_path(db)) {
        Ok(l) => l,
        Err(lock::LockError::Held(pid)) => {
            eprintln!("wrapped-collector: already running (pid {pid})");
            return 1;
        }
        Err(lock::LockError::Io(e)) => {
            eprintln!("wrapped-collector: cannot create lock file: {e}");
            return 1;
        }
    };
    // Before any other thread exists, so only the signal thread receives them.
    signals::install();

    let engine = Store::open(db)
        .and_then(|store| Engine::new(store, Arc::new(SystemClock), collectors::builtin()));
    let mut engine = match engine {
        Ok(e) => e,
        Err(e) => {
            eprintln!("wrapped-collector: cannot open {}: {e}", db.display());
            return 1;
        }
    };
    // Requests left by a previous run are meaningless now.
    wrapped_core::ctl::clear(db);
    // First-launch checklist: an existing install is marked done and left
    // alone; only an empty, undecided database waits for the user.
    let setup = match engine.begin_onboarding() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wrapped-collector: cannot decide onboarding: {e}");
            return 1;
        }
    };
    engine.set_own_app_id(own_app_id());
    engine.set_scan_hooks(scan_hooks());
    engine.set_probe(Arc::new(counters::MacProbe));
    engine.set_waker(Arc::new(|| DispatchQueue::main().exec_async(poll_scans)));
    if debug {
        engine.set_logger(log);
    }
    log(&format!(
        "collector started (pid {}), database {}, recovered {} open session(s), collectors: {}{}",
        std::process::id(),
        db.display(),
        engine.recovered_sessions(),
        engine.running_collectors().join(", "),
        if setup {
            ", SETUP MODE (nothing is recorded until setup is finished)"
        } else if engine.is_paused() {
            ", PAUSED"
        } else {
            ""
        }
    ));
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            engine,
            debug,
            db: db.to_path_buf(),
            last_tick: None,
            slept_since_tick: false,
            asleep_since: None,
            last_disk_bytes: process_disk_bytes_written().unwrap_or(0),
        })
    });

    let app = NSApplication::sharedApplication(mtm);
    // Menu-bar only: no Dock icon, no app menu.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = tray::Delegate::new(mtm, data_dir_of(db), db.to_path_buf());
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    tray::install(&delegate, mtm);
    let _sources = sources::install();
    sources::send_initial_state();
    if setup && let Err(e) = open_dashboard(db) {
        log(&format!("setup: cannot open the dashboard: {e}"));
    }
    app.run();
    // `run` only returns if the app is stopped without terminating.
    shutdown();
    0
}

fn data_dir_of(db: &Path) -> PathBuf {
    db.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// This app's identifier: the main bundle's id, or the packaged default when
/// running as a bare binary.
fn own_app_id() -> String {
    objc2_foundation::NSBundle::mainBundle()
        .bundleIdentifier()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "io.github.peaperfish.wrapped".to_string())
}

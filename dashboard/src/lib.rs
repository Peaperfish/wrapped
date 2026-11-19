//! Wrapped dashboard (DESIGN.md §9): a Tauri window over the stats engine.
//!
//! * Runs only while its window is open; closing the window quits.
//! * Reads the collector's database read-only and never writes
//!   (see [`data`]); `--db <path>` picks another database.
//! * Commands: `meta`, `summary(from, to, vsFrom?, vsTo?)`, `series(year, metric)`,
//!   `app_names(ids)`, `flush_collector` (asks a running collector to save, by
//!   running `wrapped-collector flush`; the dashboard itself never writes) and
//!   `ready` (the page has rendered: show the window). Types are in
//!   [`types`], mirrored to TypeScript in `ui/src/lib/types/`.

pub mod data;
pub mod names;
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tauri::{Manager, State, WebviewWindow};

use crate::types::{Meta, YearSeries};

const USAGE: &str = "\
Wrapped dashboard (read-only view of the collector's database)

Usage:
  wrapped-dashboard [--db <path>] [--debug]
  wrapped-dashboard --version

  --db <path>   database to show (default: WRAPPED_DB, else the collector's)
  --debug       log every command's time to stderr (also WRAPPED_DEBUG=1)
";

/// If the page never reports `ready` (a script error), show the window anyway.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

struct AppState {
    db: Option<PathBuf>,
    debug: bool,
    started: Instant,
    shown: AtomicBool,
    /// The Now tab's sampler, created on the first `now_snapshot` call; it
    /// does work only inside that call.
    now: std::sync::Mutex<Option<wrapped_core::now::NowSampler>>,
}

impl AppState {
    fn log(&self, msg: &str) {
        if self.debug {
            eprintln!(
                "[dashboard +{:.0} ms] {msg}",
                self.started.elapsed().as_secs_f64() * 1e3
            );
        }
    }

    /// Run `f`, logging how long it took.
    fn timed<T>(&self, label: &str, f: impl FnOnce() -> data::Result<T>) -> data::Result<T> {
        let t = Instant::now();
        let out = f();
        let ms = t.elapsed().as_secs_f64() * 1e3;
        match &out {
            Ok(_) => self.log(&format!("{label}: {ms:.2} ms")),
            Err(e) => self.log(&format!("{label}: error after {ms:.2} ms: {e}")),
        }
        out
    }

    fn db(&self) -> data::Result<&std::path::Path> {
        self.db
            .as_deref()
            .ok_or_else(|| "no database path on this platform; pass --db <path>".to_string())
    }
}

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

#[tauri::command(async)]
fn meta(state: State<'_, AppState>) -> data::Result<Meta> {
    state.timed("meta", || data::meta(state.db.as_deref(), today()))
}

#[tauri::command(async)]
fn summary(
    state: State<'_, AppState>,
    from: String,
    to: String,
    vs_from: Option<String>,
    vs_to: Option<String>,
) -> data::Result<Value> {
    state.timed(&format!("summary {from}..{to}"), || {
        data::summary(
            state.db()?,
            &from,
            &to,
            vs_from.as_deref(),
            vs_to.as_deref(),
        )
    })
}

#[tauri::command(async)]
fn series(state: State<'_, AppState>, year: i32, metric: String) -> data::Result<YearSeries> {
    state.timed(&format!("series {year} {metric}"), || {
        data::series(state.db()?, year, &metric)
    })
}

/// Display names for bundle ids (see [`names`]).
#[tauri::command(async)]
fn app_names(ids: Vec<String>) -> std::collections::HashMap<String, String> {
    names::app_names(&ids)
}

/// The tray's "Settings…" leaves a flag file next to the database; the page
/// asks for it at start and when the window gains focus. True (and the flag
/// removed) when Settings should open now.
#[tauri::command]
fn take_open_settings(state: State<'_, AppState>) -> bool {
    let Ok(db) = state.db() else { return false };
    std::fs::remove_file(wrapped_core::ctl::open_settings_flag(db)).is_ok()
}

/// Recorded app bundle ids, most time first (exclude list suggestions).
#[tauri::command(async)]
fn recorded_apps(state: State<'_, AppState>) -> data::Result<Vec<String>> {
    state.timed("recorded_apps", || data::recorded_apps(state.db()?))
}

/// Author emails found under `folders` (read-only, may take a few seconds).
#[tauri::command(async)]
fn git_emails(folders: Vec<String>) -> Vec<types::GitEmail> {
    data::git_emails(&folders)
}

/// Write an export to a path the user picked (JSON file or CSV folder).
#[tauri::command(async)]
fn export_data(
    state: State<'_, AppState>,
    kind: String,
    path: String,
) -> data::Result<types::ExportResult> {
    state.timed("export_data", || {
        data::export_data(state.db()?, &kind, Path::new(&path))
    })
}

/// The collector binary that goes with this dashboard: `Wrapped.app/Contents/
/// MacOS/wrapped-collector` when this is the nested app, else the one next to
/// this executable.
fn collector_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = if cfg!(windows) {
        "wrapped-collector.exe"
    } else {
        "wrapped-collector"
    };
    // <bundle>/Contents/Helpers/Wrapped Dashboard.app/Contents/MacOS/<exe>
    let nested = dir
        .ancestors()
        .nth(3)
        .filter(|p| p.file_name().is_some_and(|n| n == "Helpers"))
        .and_then(Path::parent)
        .map(|contents| contents.join("MacOS").join(name));
    nested
        .filter(|p| p.is_file())
        .or_else(|| Some(dir.join(name)).filter(|p| p.is_file()))
}

/// Ask a running collector to flush now, the same as `wrapped-collector
/// flush` (SIGUSR1). True when one was found and asked; false when no
/// collector is running (or none can be run): then there is nothing to wait for.
#[tauri::command(async)]
fn flush_collector(state: State<'_, AppState>) -> bool {
    let (Some(exe), Ok(db)) = (collector_exe(), state.db()) else {
        return false;
    };
    let asked = std::process::Command::new(exe)
        .arg("flush")
        .env("WRAPPED_DB", db)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    state.log(&format!("flush_collector: asked = {asked}"));
    asked
}

/// Send one request to the running collector (`wrapped-collector ctl`): the
/// dashboard never writes, the collector does. Always returns the reply as
/// `{ok: true, data}` or `{ok: false, code, error}`; `code` is `not_running`
/// when no collector is running and `timeout` when it did not answer.
#[tauri::command(async)]
fn ctl(state: State<'_, AppState>, cmd: String, args: Option<Value>) -> Value {
    let fail = |code: &str, error: String| serde_json::json!({ "ok": false, "code": code, "error": error });
    let (Some(exe), Ok(db)) = (collector_exe(), state.db()) else {
        return fail(
            "no_collector",
            "The Wrapped collector was not found next to the dashboard.".into(),
        );
    };
    let out = std::process::Command::new(exe)
        .arg("ctl")
        .arg(&cmd)
        .arg(args.unwrap_or(Value::Null).to_string())
        .env("WRAPPED_DB", db)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let reply = match out {
        Ok(o) => serde_json::from_slice::<Value>(&o.stdout)
            .unwrap_or_else(|_| fail("io", "The collector gave no usable answer.".into())),
        Err(e) => fail("io", format!("Cannot run the collector: {e}")),
    };
    state.log(&format!("ctl {cmd}: ok = {}", reply["ok"]));
    reply
}

/// Open the Wrapped app (the collector), for "Wrapped isn't running".
#[tauri::command(async)]
fn open_wrapped(state: State<'_, AppState>) -> bool {
    let Some(exe) = collector_exe() else {
        return false;
    };
    // <bundle>/Contents/MacOS/wrapped-collector -> <bundle>
    let bundle = exe
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|p| p.extension().is_some_and(|e| e == "app"));
    let ok = match (bundle, cfg!(target_os = "macos")) {
        (Some(b), true) => std::process::Command::new("/usr/bin/open")
            .arg(b)
            .status()
            .is_ok_and(|s| s.success()),
        _ => {
            // A bare binary: start it detached, with the same database.
            let mut c = std::process::Command::new(&exe);
            if let Ok(db) = state.db() {
                c.env("WRAPPED_DB", db);
            }
            c.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
        }
    };
    state.log(&format!("open_wrapped: {ok}"));
    ok
}

/// Title bar / window theme: "light" / "dark" force it, anything else follows
/// macOS live. Sets the app appearance and the window's own theme, and paints the
/// window background in the theme's page colour (a dark background colour under
/// the title bar would otherwise keep it dark).
#[tauri::command]
fn set_window_theme(app: tauri::AppHandle, window: WebviewWindow, theme: Option<String>) {
    let forced = match theme.as_deref() {
        Some("light") => Some(tauri::Theme::Light),
        Some("dark") => Some(tauri::Theme::Dark),
        _ => None,
    };
    app.set_theme(forced);
    let _ = window.set_theme(forced);
    let effective = window.theme().unwrap_or(tauri::Theme::Dark);
    let bg = match effective {
        tauri::Theme::Light => tauri::window::Color(0xf1, 0xf3, 0xf8, 0xff),
        _ => tauri::window::Color(0x0b, 0x0d, 0x12, 0xff),
    };
    let _ = window.set_background_color(Some(bg));
}

/// One sample of the live hardware numbers (the Now tab polls this while it
/// is visible). Rates are null on the first call.
#[tauri::command(async)]
fn now_snapshot(state: State<'_, AppState>) -> Value {
    data::now_snapshot(&state.now)
}

/// The page has rendered its first frame with data: show the window.
#[tauri::command]
fn ready(window: WebviewWindow, state: State<'_, AppState>) {
    show(&window, &state, "ready");
}

fn show(window: &WebviewWindow, state: &AppState, why: &str) {
    if state.shown.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = window.show();
    let _ = window.set_focus();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    state.log(&format!("window shown ({why}) at unix_ms={unix_ms}"));
}

/// Parse arguments and run the window. Returns the process exit code.
pub fn main() -> i32 {
    let started = Instant::now();
    let mut debug = std::env::var("WRAPPED_DEBUG").is_ok_and(|v| !v.is_empty() && v != "0");
    let mut db: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--version" | "-V" | "version" => {
                println!(
                    "wrapped-dashboard {} ({})",
                    env!("CARGO_PKG_VERSION"),
                    env!("WRAPPED_BUILD_ID")
                );
                return 0;
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return 0;
            }
            "--debug" => debug = true,
            "--db" => match args.next() {
                Some(p) if !p.is_empty() => db = Some(PathBuf::from(p)),
                _ => {
                    eprint!("wrapped-dashboard: --db needs a path\n\n{USAGE}");
                    return 2;
                }
            },
            // macOS may add its own arguments (e.g. -psn_…); ignore them.
            _ => {}
        }
    }
    let state = AppState {
        db: db.or_else(data::default_db_path),
        debug,
        started,
        shown: AtomicBool::new(false),
        now: std::sync::Mutex::new(None),
    };
    state.log(&format!(
        "start, database {}",
        state
            .db
            .as_ref()
            .map_or("(none)".into(), |p| p.display().to_string())
    ));
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            meta,
            summary,
            series,
            app_names,
            flush_collector,
            take_open_settings,
            recorded_apps,
            git_emails,
            export_data,
            ctl,
            open_wrapped,
            ready,
            now_snapshot,
            set_window_theme
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(SHOW_FALLBACK);
                if let Some(w) = handle.get_webview_window("main") {
                    show(&w, &handle.state::<AppState>(), "fallback timer");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!());
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("wrapped-dashboard: {e}");
            1
        }
    }
}

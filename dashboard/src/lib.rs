//! Wrapped dashboard (DESIGN.md §9): a Tauri window over the stats engine.
//!
//! * Runs only while its window is open; closing the window quits.
//! * Reads the collector's database read-only and never writes
//!   (see [`data`]); `--db <path>` picks another database.
//! * Commands: `meta`, `summary(from, to)`, `series(year, metric)` and
//!   `ready` (the page has rendered: show the window). Types are in
//!   [`types`], mirrored to TypeScript in `ui/src/lib/types/`.

pub mod data;
pub mod types;

use std::path::PathBuf;
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
fn summary(state: State<'_, AppState>, from: String, to: String) -> data::Result<Value> {
    state.timed(&format!("summary {from}..{to}"), || {
        data::summary(state.db()?, &from, &to)
    })
}

#[tauri::command(async)]
fn series(state: State<'_, AppState>, year: i32, metric: String) -> data::Result<YearSeries> {
    state.timed(&format!("series {year} {metric}"), || {
        data::series(state.db()?, year, &metric)
    })
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
    };
    state.log(&format!(
        "start, database {}",
        state
            .db
            .as_ref()
            .map_or("(none)".into(), |p| p.display().to_string())
    ));
    let result = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![meta, summary, series, ready])
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

//! Command line. With no command it runs the collector (tray app); the other
//! commands are temporary stand-ins for what the UI will do later.

use std::path::PathBuf;

use chrono::NaiveDate;
use wrapped_core::collector::Unit;
use wrapped_core::store::MetricRow;
use wrapped_core::time::{MS_PER_DAY, MS_PER_MIN, local_midnight_ms};
use wrapped_core::{Clock, Store, SystemClock, collectors};

use crate::platform;

const USAGE: &str = "\
wrapped-collector: local-only usage tracker (Milestone 1)

USAGE
  wrapped-collector [--debug]               run the collector (tray icon)
  wrapped-collector settings get [<key>]    show one or all settings
  wrapped-collector settings set <key> <value>
  wrapped-collector stats [today|YYYY-MM-DD]     daily metrics for a local day
  wrapped-collector sessions [today|YYYY-MM-DD]  session rows for a local day
  wrapped-collector export --json <file>    everything, one JSON file
  wrapped-collector export --csv <dir>      everything, one CSV per table
  wrapped-collector --version               print crate version and build (git hash)
  wrapped-collector flush                 ask the running collector to flush now
  wrapped-collector login enable|disable|status   launch at login (macOS 13+, .app only)

SETTINGS (values are JSON; a bare word is taken as a string)
  idle_threshold_secs                 180
  active_without_input.apps           '[\"com.apple.Preview\"]'
  active_without_input.timeout_secs   1800
  exclude.apps                        '[\"com.example.Secret\"]'
  pause                               off | indefinite | '{\"until_ms\": <utc ms>}'
  collector.<name>.enabled            true | false   (app_usage, active_idle)

ENVIRONMENT
  WRAPPED_DEBUG=1   same as --debug: log every system event, tick gap and flush to stderr
  WRAPPED_DB=path   use another database file (default: <data dir>/wrapped.db)
";

pub fn main() -> i32 {
    let mut debug = std::env::var("WRAPPED_DEBUG").is_ok_and(|v| !v.is_empty() && v != "0");
    let owned: Vec<String> = std::env::args().skip(1).collect();
    let mut args: Vec<&str> = Vec::new();
    for a in &owned {
        if a == "--debug" {
            debug = true;
        } else {
            args.push(a);
        }
    }
    if matches!(args.as_slice(), ["--version" | "-V" | "version"]) {
        println!("{}", version_line());
        return 0;
    }
    let Some(db) = db_path() else {
        eprintln!("wrapped-collector: no data directory on this platform; set WRAPPED_DB");
        return 1;
    };
    let result = match args.as_slice() {
        [] | ["run"] => return platform::run_collector(&db, debug),
        ["help" | "--help" | "-h"] => {
            print!("{USAGE}");
            return 0;
        }
        ["settings", "get"] => settings_get(&db, None),
        ["settings", "get", key] => settings_get(&db, Some(key)),
        ["settings", "set", key, value] => settings_set(&db, key, value),
        ["stats"] => stats(&db, "today"),
        ["stats", day] => stats(&db, day),
        ["sessions"] => sessions(&db, "today"),
        ["sessions", day] => sessions(&db, day),
        ["export", "--json", path] => export(&db, true, path),
        ["export", "--csv", path] => export(&db, false, path),
        ["flush"] => match platform::request_flush(&db) {
            Some(pid) => {
                println!("asked collector (pid {pid}) to flush");
                Ok(())
            }
            None => Err("no running collector found".into()),
        },
        ["login", cmd @ ("enable" | "disable" | "status")] => return platform::login(cmd),
        _ => {
            eprint!("{USAGE}");
            return 2;
        }
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("wrapped-collector: {e}");
            1
        }
    }
}

/// `wrapped-collector <crate version> (<build id>)`; the build id is the short
/// git hash, plus `-dirty` for uncommitted changes.
fn version_line() -> String {
    format!(
        "wrapped-collector {} ({})",
        env!("CARGO_PKG_VERSION"),
        env!("WRAPPED_BUILD_ID")
    )
}

type CliResult = Result<(), Box<dyn std::error::Error>>;

pub fn db_path() -> Option<PathBuf> {
    match std::env::var_os("WRAPPED_DB") {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => platform::data_dir().map(|d| d.join("wrapped.db")),
    }
}

fn settings_get(db: &std::path::Path, key: Option<&str>) -> CliResult {
    let store = Store::open(db)?;
    let (s, skipped) = store.load_settings()?;
    for msg in skipped {
        eprintln!("warning: ignored stored setting: {msg}");
    }
    match key {
        Some(k) => println!("{}", s.get_json(k)?),
        None => {
            for (k, v) in s.to_rows() {
                println!("{k} = {v}");
            }
            for c in collectors::builtin() {
                let i = c.info();
                println!(
                    "collector.{}.enabled = {}",
                    i.name,
                    s.collector_enabled(i.name, i.default_on)
                );
            }
        }
    }
    Ok(())
}

fn settings_set(db: &std::path::Path, key: &str, value: &str) -> CliResult {
    if let Some(name) = key
        .strip_prefix("collector.")
        .and_then(|r| r.strip_suffix(".enabled"))
        && !collectors::builtin().iter().any(|c| c.info().name == name)
    {
        return Err(format!("unknown collector `{name}`").into());
    }
    let store = Store::open(db)?;
    let stored = store.set_setting(key, value)?;
    println!("{key} = {stored}");
    if platform::running_pid(db).is_some() {
        println!("(the running collector picks this up at its next minute tick)");
    }
    Ok(())
}

fn parse_day(day: &str) -> Result<NaiveDate, Box<dyn std::error::Error>> {
    if day == "today" {
        return Ok(SystemClock.now().local_date());
    }
    Ok(NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map_err(|_| format!("expected `today` or YYYY-MM-DD, got `{day}`"))?)
}

/// Ask a running collector to flush and give it a moment, so reads are fresh.
fn fresh_store(db: &std::path::Path) -> Result<Store, Box<dyn std::error::Error>> {
    if platform::request_flush(db).is_some() {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(Store::open(db)?)
}

fn fmt_secs(v: f64) -> String {
    let s = v.round() as i64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

fn stats(db: &std::path::Path, day: &str) -> CliResult {
    let date = parse_day(day)?;
    let store = fresh_store(db)?;
    let rows = store.daily_metrics(date)?;
    println!("daily_metrics for {date} (local day)");
    if rows.is_empty() {
        println!("  (no data)");
        return Ok(());
    }
    let defs: Vec<_> = collectors::builtin()
        .iter()
        .flat_map(|c| {
            let name = c.info().name;
            c.metrics().into_iter().map(move |m| (name, m))
        })
        .collect();
    let mut last: Option<(&str, &str)> = None;
    for MetricRow {
        collector,
        metric,
        key,
        value,
        ..
    } in &rows
    {
        let unit = defs
            .iter()
            .find(|(c, m)| c == collector && m.id == metric)
            .map(|(_, m)| m.unit);
        let shown = match unit {
            Some(Unit::Seconds) => format!("{value:>10.0}  {}", fmt_secs(*value)),
            _ => format!("{value:>10}"),
        };
        if last.is_none_or(|(c, _)| c != collector) {
            println!("{collector}");
        }
        if key.is_empty() {
            println!("  {metric:<24} {shown}");
        } else {
            if last != Some((collector, metric)) {
                println!("  {metric}");
            }
            println!("    {key:<38} {shown}");
        }
        last = Some((collector, metric));
    }
    Ok(())
}

fn sessions(db: &std::path::Path, day: &str) -> CliResult {
    let date = parse_day(day)?;
    let store = fresh_store(db)?;
    // Generous UTC window around the local day; filter by local start date.
    let day0 = local_midnight_ms(date);
    let rows = store.sessions(
        None,
        day0 - 15 * 60 * MS_PER_MIN,
        day0 + MS_PER_DAY + 15 * 60 * MS_PER_MIN,
    )?;
    println!("sessions overlapping {date} (local times; * = open, end = last flush)");
    println!(
        "{:<25} {:<25} {:>11}  {:<12} key",
        "start", "end", "duration", "collector"
    );
    for r in rows
        .iter()
        .filter(|r| r.start.local_date() <= date && r.end.plus_ms(-1).local_date() >= date)
    {
        println!(
            "{:<25} {:<25} {:>11}  {:<12} {}{}",
            r.start.to_rfc3339(),
            r.end.to_rfc3339(),
            fmt_secs((r.end.utc_ms - r.start.utc_ms) as f64 / 1000.0),
            r.collector,
            r.key,
            if r.open { " *" } else { "" }
        );
    }
    Ok(())
}

fn export(db: &std::path::Path, json: bool, path: &str) -> CliResult {
    let store = fresh_store(db)?;
    let c = if json {
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        store.export_json(&mut w, SystemClock.now())?
    } else {
        store.export_csv(std::path::Path::new(path))?
    };
    println!(
        "exported {} sessions, {} daily metric rows, {} samples, {} settings to {path}",
        c.sessions, c.daily_metrics, c.samples, c.settings
    );
    Ok(())
}

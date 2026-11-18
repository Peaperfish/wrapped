//! Command line. With no command it runs the collector (tray app); the other
//! commands are temporary stand-ins for what the UI will do later.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::NaiveDate;
use wrapped_core::collector::{Kind, Unit};
use wrapped_core::store::MetricRow;
use wrapped_core::time::{MS_PER_DAY, MS_PER_MIN, local_midnight_ms};
use wrapped_core::{Clock, Engine, Store, SystemClock, collectors};

use crate::platform;

const USAGE: &str = "\
wrapped-collector: local-only usage tracker (Milestone 4)

USAGE
  wrapped-collector [--debug]               run the collector (tray icon)
  wrapped-collector settings get [<key>]    show one or all settings
  wrapped-collector settings set <key> <value>
  wrapped-collector stats [today|YYYY-MM-DD]     daily metrics for a local day
  wrapped-collector stats range <from> <to> [--json]
                                            totals, top keys, change vs the previous period,
                                            time of day, per-day series and records for an
                                            inclusive range of local dates (read-only)
  wrapped-collector sessions [today|YYYY-MM-DD]  session rows for a local day
  wrapped-collector export --json <file>    everything, one JSON file
  wrapped-collector export --csv <dir>      everything, one CSV per table
  wrapped-collector --version               print crate version and build (git hash)
  wrapped-collector scan [git|photos|all]   scan now (enabled scanned collectors; default all)
  wrapped-collector git emails             list author emails in git.folders (read-only)
  wrapped-collector probe disk-memory      read disk / memory counters twice, 60 s apart (stores nothing)
  wrapped-collector flush                 ask the running collector to flush now
  wrapped-collector login enable|disable|status   launch at login (macOS 13+, .app only)

SETTINGS (values are JSON; a bare word is taken as a string)
  idle_threshold_secs                 180
  active_without_input.apps           '[\"com.apple.Preview\"]'
  active_without_input.timeout_secs   1800
  exclude.apps                        '[\"com.example.Secret\"]'
  pause                               off | indefinite | '{\"until_ms\": <utc ms>}'
  collector.<name>.enabled            true | false   (app_usage, active_idle, git, photos, disk_memory)
  git.folders                         '[\"~/code\"]'
  git.author_emails                   '[\"me@example.com\"]'  (set from git config when git is turned on)
  photos.folders                      '[\"~/Pictures\"]'
  photos.extensions                   '[\"jpg\",\"jpeg\",\"heic\",\"heif\",\"arw\",\"dng\"]'

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
        ["stats", "range", from, to] => stats_range(&db, from, to, false),
        ["stats", "range", from, to, "--json"] => stats_range(&db, from, to, true),
        ["stats"] => stats(&db, "today"),
        ["stats", day] => stats(&db, day),
        ["sessions"] => sessions(&db, "today"),
        ["sessions", day] => sessions(&db, day),
        ["git", "emails"] => git_emails(&db),
        ["probe", "disk-memory"] => probe_disk_memory(),
        ["scan"] => scan(&db, "all"),
        ["scan", what] => scan(&db, what),
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
    // Turning a collector on stores its defaults explicitly (git: author emails).
    if stored == "true"
        && let Some(name) = key
            .strip_prefix("collector.")
            .and_then(|r| r.strip_suffix(".enabled"))
        && let Some(c) = collectors::builtin()
            .into_iter()
            .find(|c| c.info().name == name)
    {
        let (s, _) = store.load_settings()?;
        for (k, v) in c.enable_defaults(&s) {
            let stored = store.set_setting(&k, &v)?;
            println!("{k} = {stored}");
            if stored == "[]" {
                println!(
                    "  (nothing found; set it with: settings set {k} '[\"you@example.com\"]')"
                );
            }
        }
    }
    let running = platform::running_pid(db).is_some();
    // A changed counting rule (git author emails) resets that collector's data
    // so the next scan reads its full history again.
    let (s, _) = store.load_settings()?;
    let mut store = store;
    for c in collectors::builtin() {
        let i = c.info();
        if !s.collector_enabled(i.name, i.default_on) {
            continue;
        }
        let Some(fp) = c.scan_fingerprint(&s) else {
            continue;
        };
        if store.scan_fingerprint(i.name)?.as_deref() == Some(fp.as_str()) {
            continue;
        }
        if running {
            println!(
                "({}: the running collector resets its data and rescans everything at its next tick)",
                i.name
            );
        } else if store.sync_scan_fingerprint(i.name, &fp)? {
            println!(
                "({}: data reset; the next scan reads the full history)",
                i.name
            );
        }
    }
    if running {
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
            Some(Unit::Bytes) => format!("{value:>14.0}  {}", fmt_bytes(*value)),
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

/// Stats for an inclusive range of local dates. Opens the database read-only
/// (no migration, no writes); a collector running on it is asked to flush
/// first, as `stats` does.
fn stats_range(db: &std::path::Path, from: &str, to: &str, json: bool) -> CliResult {
    use wrapped_core::stats::{DateRange, SUMMARY_TOP_N, Stats, hidden_collectors};
    let range = DateRange::new(parse_day(from)?, parse_day(to)?)?;
    if !db.exists() {
        return Err(format!("no database at {}", db.display()).into());
    }
    if platform::request_flush(db).is_some() {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let store = Store::open_read_only(db)?;
    let (settings, _) = store.load_settings()?;
    let all = collectors::builtin();
    let st = Stats::new(&store, &all, &settings);
    let sum = st.summary(range, SUMMARY_TOP_N)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&sum.to_json())?);
        return Ok(());
    }
    let hidden = hidden_collectors(&all, &settings);
    print_summary(&sum, &st, &hidden);
    Ok(())
}

fn fmt_value(v: f64, unit: Unit) -> String {
    match unit {
        Unit::Seconds => fmt_secs(v),
        Unit::Bytes => fmt_bytes(v),
        Unit::Count if v.fract() == 0.0 => format!("{v:.0}"),
        Unit::Count => format!("{v:.2}"),
    }
}

fn fmt_pct(p: Option<f64>) -> String {
    match p {
        Some(p) => format!("{p:+.1}%"),
        None => "new".to_string(),
    }
}

/// 24 hourly values as one line of block characters.
fn sparkline(hours: &[f64; 24]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = hours.iter().copied().fold(0.0, f64::max);
    hours
        .iter()
        .map(|h| {
            if max <= 0.0 || *h <= 0.0 {
                ' '
            } else {
                BARS[((h / max) * 7.0).round() as usize]
            }
        })
        .collect()
}

fn print_summary(
    sum: &wrapped_core::stats::Summary,
    st: &wrapped_core::stats::Stats<'_>,
    hidden: &std::collections::BTreeSet<&'static str>,
) {
    let r = sum.range;
    println!(
        "Stats for {} .. {} ({} day{}), compared with {} .. {}",
        r.from,
        r.to,
        r.days(),
        if r.days() == 1 { "" } else { "s" },
        sum.previous.from,
        sum.previous.to
    );
    if !hidden.is_empty() {
        let h: Vec<&str> = hidden.iter().copied().collect();
        println!("Hidden (collector off): {}", h.join(", "));
    }
    let mut collectors: Vec<&str> = Vec::new();
    for m in st.metrics() {
        if !collectors.contains(&m.collector) {
            collectors.push(m.collector);
        }
    }
    for c in collectors {
        println!();
        println!("{c}");
        let mut any = false;
        for cmp in sum.comparisons.iter().filter(|x| x.collector == c) {
            any = true;
            println!(
                "  {:<24} {:>16}   previous {:>16}   {}",
                cmp.metric,
                fmt_value(cmp.current, cmp.unit),
                fmt_value(cmp.previous, cmp.unit),
                fmt_pct(cmp.pct)
            );
        }
        for (m, keys) in sum.top.iter().filter(|(m, _)| m.collector == c) {
            if keys.is_empty() {
                continue;
            }
            any = true;
            println!("  top {}", m.def.id);
            for (i, (k, v)) in keys.iter().enumerate() {
                println!("    {}. {:<36} {:>14}", i + 1, k, fmt_value(*v, m.def.unit));
            }
        }
        for (m, days) in sum.series.iter().filter(|(m, _)| m.collector == c) {
            let active = days.iter().filter(|d| d.1 > 0.0).count();
            if active == 0 {
                continue;
            }
            any = true;
            let best = days
                .iter()
                .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
                .expect("non-empty");
            println!(
                "  per day {:<20} {active} of {} days > 0, highest {} ({})",
                m.def.id,
                days.len(),
                best.0,
                fmt_value(best.1, m.def.unit)
            );
        }
        if !any {
            println!("  (no data)");
        }
    }
    let profiles: Vec<_> = sum
        .time_of_day
        .iter()
        .filter(|p| p.hours.iter().any(|h| *h > 0.0))
        .collect();
    if !profiles.is_empty() {
        println!();
        println!("Time of day (local hours 00..23)");
        for p in profiles {
            let peak = p
                .hours
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(0, |x| x.0);
            println!(
                "  {:<34} |{}|  peak {peak:02}:00",
                format!("{}/{}", p.collector, p.metric),
                sparkline(&p.hours)
            );
        }
    }
    println!();
    println!("Records");
    let rec = &sum.records;
    let mut none = true;
    if let Some(l) = &rec.longest_session {
        none = false;
        println!(
            "  longest app session   {} ({}, {} .. {})",
            fmt_secs(l.secs),
            l.app,
            l.start.to_rfc3339(),
            l.end.to_rfc3339()
        );
    }
    if let Some(n) = &rec.latest_night {
        none = false;
        println!(
            "  latest night          {} at {:02}:{:02} ({})",
            n.night,
            n.clock_min / 60 % 24,
            n.clock_min % 60,
            n.at.to_rfc3339()
        );
    }
    for (name, s) in [
        ("longest active streak", rec.active_streak),
        ("longest commit streak", rec.commit_streak),
    ] {
        if let Some(s) = s {
            none = false;
            println!(
                "  {name}  {} day{} ({} .. {})",
                s.days,
                if s.days == 1 { "" } else { "s" },
                s.from,
                s.to
            );
        }
    }
    if none {
        println!("  (none)");
    }
}

/// Every author email in the configured git folders, for choosing
/// `git.author_emails`. Prints only; stores nothing.
fn git_emails(db: &std::path::Path) -> CliResult {
    let (s, _) = Store::open(db)?.load_settings()?;
    let mine = s.git_author_emails.clone().unwrap_or_default();
    println!(
        "author emails in {} (* = in git.author_emails; commits counted as scans count them)",
        s.git_folders.join(", ")
    );
    let report = collectors::git::author_email_report(&s.git_folders);
    let width = report
        .iter()
        .map(|r| r.email.len())
        .max()
        .unwrap_or(5)
        .max(5);
    println!("  {:<width$} {:>8}  repos", "email", "commits");
    for r in &report {
        let mark = if mine.contains(&r.email) { '*' } else { ' ' };
        let repos: Vec<&str> = r.repos.iter().map(String::as_str).collect();
        println!(
            "{mark} {:<width$} {:>8}  {}",
            r.email,
            r.commits,
            repos.join(", ")
        );
    }
    for m in mine
        .iter()
        .filter(|m| !report.iter().any(|r| &r.email == *m))
    {
        println!("* {m:<width$} {:>8}  (not found)", 0);
    }
    Ok(())
}

fn scan(db: &std::path::Path, what: &str) -> CliResult {
    let scanned: Vec<&'static str> = collectors::builtin()
        .iter()
        .filter(|c| c.kind() == Kind::Scanned)
        .map(|c| c.info().name)
        .collect();
    let names: Vec<&'static str> = match what {
        "all" => scanned.clone(),
        n => match scanned.iter().find(|s| **s == n) {
            Some(s) => vec![*s],
            None => {
                return Err(format!("expected one of {} or all", scanned.join(", ")).into());
            }
        },
    };
    if let Some(pid) = platform::running_pid(db) {
        let store = Store::open(db)?;
        for n in &names {
            store.request_scan(n, SystemClock.now())?;
        }
        println!(
            "a collector is running on this database (pid {pid}); it starts the scan at its next \
             minute tick and logs the result"
        );
        return Ok(());
    }
    let _lock = platform::hold_db_lock(db)?;
    let mut engine = Engine::new(
        Store::open(db)?,
        Arc::new(SystemClock),
        collectors::builtin(),
    )?;
    engine.set_auto_scans(false);
    engine.set_scan_hooks(platform::scan_hooks());
    let t0 = Instant::now();
    let mut started = 0;
    for n in &names {
        let explicit = what != "all";
        match engine.start_scan(n) {
            Ok(_) => started += 1,
            Err(e) if explicit => return Err(e.into()),
            Err(e) => println!("{n}: skipped ({e})"),
        }
    }
    if started == 0 {
        return Ok(());
    }
    let reports = engine.wait_scans()?;
    engine.shutdown()?;
    let wall = t0.elapsed().as_secs_f64();
    for r in &reports {
        println!(
            "{}: {} in {:.1}s",
            r.collector,
            if r.completed {
                "completed"
            } else {
                "stopped early"
            },
            (r.finished.utc_ms - r.started.utc_ms) as f64 / 1000.0
        );
        for (k, v) in &r.counters {
            println!("  {k:<22} {v}");
        }
        println!(
            "  {:<22} {}",
            "items stored (total)",
            engine.store().item_count(r.collector)?
        );
    }
    println!("process");
    println!("  {:<22} {wall:.2}", "wall time (s)");
    if let Some(u) = platform::usage() {
        println!("  {:<22} {:.2}", "cpu time (s)", u.cpu_secs);
        println!(
            "  {:<22} {:.2}",
            "child cpu time (s)",
            platform::children_cpu_secs()
        );
        println!("  {:<22} {}", "disk bytes read", u.disk_bytes_read);
        println!("  {:<22} {}", "disk bytes written", u.disk_bytes_written);
        println!(
            "  {:<22} {:.1} MB",
            "peak phys footprint",
            u.peak_footprint as f64 / 1_048_576.0
        );
    }
    let size = |p: PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    println!(
        "  {:<22} {} (+ WAL {})",
        "database bytes",
        size(db.to_path_buf()),
        size(db.with_extension("db-wal"))
    );
    Ok(())
}

/// Two readings of the disk & memory counters 60 s apart: raw values and
/// deltas. Read-only: opens no database and stores nothing.
fn probe_disk_memory() -> CliResult {
    use wrapped_core::collectors::disk_memory::deltas;
    let interval = std::time::Duration::from_secs(60);
    let read = || platform::disk_memory().ok_or("disk / memory counters not available here");
    let t0 = SystemClock.now();
    let a = read()?;
    println!("reading 1 at {}", t0.to_rfc3339());
    print_reading(&a);
    println!("waiting {} s ...", interval.as_secs());
    std::thread::sleep(interval);
    let t1 = SystemClock.now();
    let b = read()?;
    println!("reading 2 at {}", t1.to_rfc3339());
    print_reading(&b);
    let wall = t1.utc_ms - t0.utc_ms;
    println!("deltas over {:.1} s", wall as f64 / 1000.0);
    match deltas(&a, &b, wall) {
        Some([r, w, si, so]) => {
            for (k, v) in [
                ("disk bytes read", r),
                ("disk bytes written", w),
                ("swap in bytes", si),
                ("swap out bytes", so),
            ] {
                println!("  {k:<22} {v:>16}  {}", fmt_bytes(v as f64));
            }
            let dc = b.compressed_bytes as i64 - a.compressed_bytes as i64;
            println!("  {:<22} {dc:>+16}", "compressed bytes");
        }
        None => println!("  counters restarted between readings (reboot or reset); no delta"),
    }
    Ok(())
}

fn print_reading(r: &wrapped_core::probe::DiskMemoryReading) {
    let boot = wrapped_core::Timestamp::new(r.boot_time_ms, SystemClock.now().offset_min);
    println!("  {:<22} {}", "boot time", boot.to_rfc3339());
    for (k, v) in [
        ("disk bytes read", r.disk_bytes_read),
        ("disk bytes written", r.disk_bytes_written),
        ("swap in bytes", r.swap_in_bytes),
        ("swap out bytes", r.swap_out_bytes),
        ("compressed bytes", r.compressed_bytes),
    ] {
        println!("  {k:<22} {v:>16}  {}", fmt_bytes(v as f64));
    }
    println!("  {:<22} {:>16}", "memory pressure", r.pressure.key());
}

fn fmt_bytes(v: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = v;
    let mut i = 0;
    while v.abs() >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.2} {}", UNITS[i])
    }
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
        "exported {} sessions, {} daily metric rows, {} samples, {} settings, {} items to {path}",
        c.sessions, c.daily_metrics, c.samples, c.settings, c.items
    );
    Ok(())
}

//! SQLite storage: the shared batched writer's back end, daily rollups,
//! settings and data deletion.
//!
//! Journal settings (chosen for the ~1–2 MB/day SSD budget):
//! * `journal_mode=WAL`: a commit appends only the changed pages (plus a 24-byte
//!   frame header each) to the WAL. A rollback journal would also write the
//!   original pages to the journal and fsync several times per commit.
//! * `synchronous=NORMAL`: in WAL mode this fsyncs only at checkpoints, not per
//!   commit, and cannot corrupt the database. A power cut can lose the last
//!   commit(s), which is within the "lose at most ~10 min" crash budget anyway.
//! * Default 4 KB pages (the SSD/APFS block size; smaller pages would not
//!   reduce physical writes) and auto-checkpoint every 1000 pages. Because
//!   most flushes rewrite the same few "today" pages, a checkpoint writes far
//!   fewer pages than the WAL holds.
//! * `journal_size_limit` keeps the WAL file from staying large after a
//!   checkpoint.
//!
//! WITHOUT ROWID tables clustered on (collector, start) and (date, collector,
//! ...) plus diff-only metric updates keep a typical flush to ~3–4 pages.

pub mod export;
pub mod migrations;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::collector::{
    DayInput, DayItem, DaySample, FileStamp, ItemRecord, MAX_SAMPLE_WINDOW_MS, MetricValue,
    OpenSession, SampleRecord, ScanUpdate, SessionRecord, Slice,
};
use crate::settings::{Settings, SettingsError, collector_enabled_key};
use crate::time::{MS_PER_DAY, MS_PER_HOUR, Timestamp, date_from_local_ms, local_midnight_ms};

/// Local offsets in use worldwide span UTC-12..UTC+14; pad to be safe.
const OFFSET_MARGIN_MS: i64 = 15 * MS_PER_HOUR;
const KEY_MAX_SESSION: &str = "internal.max_session_ms";
const SCAN_REQUEST_PREFIX: &str = "internal.scan_request.";
const SCAN_FINGERPRINT_PREFIX: &str = "internal.scan_fingerprint.";

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Settings(SettingsError),
    NewerSchema { found: usize, supported: usize },
    Io(std::io::Error),
    Scan(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "database error: {e}"),
            Error::Settings(e) => write!(f, "{e}"),
            Error::NewerSchema { found, supported } => write!(
                f,
                "database schema version {found} is newer than this build supports ({supported}); \
                 refusing to touch it"
            ),
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Scan(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sqlite(e)
    }
}
impl From<SettingsError> for Error {
    fn from(e: SettingsError) -> Self {
        Error::Settings(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Computes one collector's metrics for one day. `None` = unknown collector.
pub type RollupFn<'a> = dyn Fn(&str, &DayInput<'_>) -> Option<Vec<MetricValue>> + 'a;

/// What one flush did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FlushReport {
    pub sessions_written: usize,
    pub sessions_deleted: usize,
    pub samples_written: usize,
    pub days_recomputed: usize,
    pub metric_rows_changed: usize,
    /// New items stored (duplicates of stored ids are not counted).
    pub items_written: usize,
    /// `scan_state` / `scan_files` rows written or deleted.
    pub scan_rows_changed: usize,
}

/// Everything one flush writes, in one transaction.
#[derive(Debug, Default, Clone, Copy)]
pub struct Batch<'a> {
    pub closed: &'a [SessionRecord],
    pub open: &'a [(&'static str, OpenSession)],
    pub samples: &'a [SampleRecord],
    pub items: &'a [ItemRecord],
    pub scans: &'a [(&'static str, ScanUpdate)],
}

/// A collector's stored scan bookkeeping.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanInfo {
    /// Start of the last completed scan (UTC ms).
    pub last_scanned_ms: Option<i64>,
    pub state: Option<String>,
}

/// One stored session row.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub collector: String,
    pub key: String,
    pub start: Timestamp,
    pub end: Timestamp,
    pub open: bool,
}

/// One `daily_metrics` row.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricRow {
    pub date: String,
    pub collector: String,
    pub metric: String,
    pub key: String,
    pub value: f64,
}

pub struct Store {
    conn: Connection,
    /// Longest session ever stored, bounding how far back a day's rollup looks.
    max_session_ms: i64,
    /// End written for each open session at the last flush, keyed by
    /// (collector, start_ms). If the session later closes earlier (idle is
    /// backdated), the days between still need recomputing.
    flushed_open: HashMap<(String, i64), i64>,
}

impl Store {
    /// Open (creating if needed) the database file and bring the schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "journal_size_limit", 1_048_576)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations::migrate(&mut conn)?;
        let max_session_ms = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [KEY_MAX_SESSION],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        Ok(Self {
            conn,
            max_session_ms,
            flushed_open: HashMap::new(),
        })
    }

    pub fn schema_version(&self) -> Result<usize> {
        migrations::schema_version(&self.conn)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Crash recovery: sessions still marked open were running when the
    /// process died. Their `end` is the last flush time; just close them.
    pub fn close_open_sessions(&mut self) -> Result<usize> {
        let n = self
            .conn
            .execute("UPDATE sessions SET open = 0 WHERE open = 1", [])?;
        self.flushed_open.clear();
        Ok(n)
    }

    // ---------- settings ----------

    /// Load settings; rows that fail validation are skipped (returned as messages).
    pub fn load_settings(&self) -> Result<(Settings, Vec<String>)> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(Settings::from_rows(
            rows.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        ))
    }

    /// Validate and store one setting. Returns the normalized JSON stored.
    pub fn set_setting(&self, key: &str, raw: &str) -> Result<String> {
        let (mut s, _) = self.load_settings()?;
        s.apply(key, raw)?;
        let value = s.get_json(key)?;
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(value)
    }

    // ---------- collector data control ----------

    /// Turn a collector off; optionally delete everything it recorded.
    /// (The running engine must stop the collector first: see `Engine::disable_collector`.)
    pub fn disable_collector(&mut self, collector: &str, keep_data: bool) -> Result<()> {
        self.set_setting(&collector_enabled_key(collector), "false")?;
        if !keep_data {
            self.delete_collector_data(collector)?;
        }
        Ok(())
    }

    /// Delete every row a collector produced (sessions, samples, items, daily
    /// metrics, scan state, per-file cache) and any pending scan request. Its
    /// on/off setting and other settings are left alone.
    pub fn delete_collector_data(&mut self, collector: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        for table in [
            "sessions",
            "samples",
            "items",
            "daily_metrics",
            "scan_state",
            "scan_files",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE collector = ?1"),
                [collector],
            )?;
        }
        tx.execute(
            "DELETE FROM settings WHERE key = ?1",
            [format!("{SCAN_REQUEST_PREFIX}{collector}")],
        )?;
        tx.commit()?;
        self.flushed_open.retain(|(c, _), _| c != collector);
        Ok(())
    }

    // ---------- the batched writer ----------

    // ---------- scanned collectors ----------

    pub fn scan_info(&self, collector: &str) -> Result<ScanInfo> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_scanned_ms, state FROM scan_state WHERE collector = ?1",
                [collector],
                |r| {
                    Ok(ScanInfo {
                        last_scanned_ms: r.get(0)?,
                        state: r.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// The per-file cache of a collector.
    pub fn scan_files(&self, collector: &str) -> Result<Vec<FileStamp>> {
        let mut st = self
            .conn
            .prepare("SELECT path_hash, size, mtime_ms FROM scan_files WHERE collector = ?1")?;
        let rows = st
            .query_map([collector], |r| {
                Ok(FileStamp {
                    path_hash: r.get::<_, i64>(0)? as u64,
                    size: r.get::<_, i64>(1)? as u64,
                    mtime_ms: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// The stored settings fingerprint of a scanned collector (see
    /// `Collector::scan_fingerprint`).
    pub fn scan_fingerprint(&self, collector: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [format!("{SCAN_FINGERPRINT_PREFIX}{collector}")],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_scan_fingerprint(&self, collector: &str, fp: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("{SCAN_FINGERPRINT_PREFIX}{collector}"), fp],
        )?;
        Ok(())
    }

    /// If the fingerprint differs from the stored one (or none is stored),
    /// delete the collector's data and incremental state and store the new
    /// fingerprint. Returns true if there was anything to reset. For use when
    /// no engine runs on this database (an engine does this itself).
    pub fn sync_scan_fingerprint(&mut self, collector: &str, fp: &str) -> Result<bool> {
        let prev = self.scan_fingerprint(collector)?;
        if prev.as_deref() == Some(fp) {
            return Ok(false);
        }
        let had_data = prev.is_some()
            || self.item_count(collector)? > 0
            || self.scan_info(collector)? != ScanInfo::default();
        self.delete_collector_data(collector)?;
        self.set_scan_fingerprint(collector, fp)?;
        Ok(had_data)
    }

    /// Ask a running collector (another process) to scan `collector` soon.
    pub fn request_scan(&self, collector: &str, now: Timestamp) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                format!("{SCAN_REQUEST_PREFIX}{collector}"),
                now.utc_ms.to_string()
            ],
        )?;
        Ok(())
    }

    /// Pending scan requests (removed as they are returned).
    pub fn take_scan_requests(&self) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare("SELECT key FROM settings WHERE key LIKE 'internal.scan_request.%'")?;
        let keys: Vec<String> = st
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for k in &keys {
            self.conn
                .execute("DELETE FROM settings WHERE key = ?1", [k])?;
        }
        Ok(keys
            .into_iter()
            .filter_map(|k| k.strip_prefix(SCAN_REQUEST_PREFIX).map(str::to_string))
            .collect())
    }

    pub fn item_count(&self, collector: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM items WHERE collector = ?1",
            [collector],
            |r| r.get(0),
        )?)
    }

    /// Write one batch in a single transaction: closed sessions, the current
    /// open sessions (with `end = now`, marked open), samples, items and scan
    /// bookkeeping, then recompute daily metrics for exactly the (collector,
    /// local day) pairs touched.
    pub fn flush(
        &mut self,
        batch: &Batch<'_>,
        now: Timestamp,
        rollup: &RollupFn<'_>,
    ) -> Result<FlushReport> {
        let Batch {
            closed,
            open,
            samples,
            items,
            scans,
        } = *batch;
        let mut report = FlushReport::default();
        let mut touched: BTreeSet<(String, NaiveDate)> = BTreeSet::new();
        let mut max_session = self.max_session_ms;
        let tx = self.conn.transaction()?;

        for rec in closed {
            let id = (rec.collector.to_string(), rec.start.utc_ms);
            let prev_end = self.flushed_open.remove(&id);
            if rec.end.utc_ms <= rec.start.utc_ms {
                report.sessions_deleted += tx.execute(
                    "DELETE FROM sessions WHERE collector = ?1 AND start_ms = ?2",
                    params![rec.collector, rec.start.utc_ms],
                )?;
            } else {
                upsert_session(&tx, rec.collector, &rec.key, rec.start, rec.end, false)?;
                report.sessions_written += 1;
                max_session = max_session.max(rec.end.utc_ms - rec.start.utc_ms);
            }
            let reach = prev_end.unwrap_or(rec.end.utc_ms).max(rec.end.utc_ms);
            touch_days(&mut touched, rec.collector, rec.start, reach);
        }

        for (collector, o) in open {
            if now.utc_ms <= o.start.utc_ms {
                continue;
            }
            upsert_session(&tx, collector, &o.key, o.start, now, true)?;
            report.sessions_written += 1;
            max_session = max_session.max(now.utc_ms - o.start.utc_ms);
            let id = (collector.to_string(), o.start.utc_ms);
            let reach = self
                .flushed_open
                .get(&id)
                .copied()
                .unwrap_or(now.utc_ms)
                .max(now.utc_ms);
            self.flushed_open.insert(id, now.utc_ms);
            touch_days(&mut touched, collector, o.start, reach);
        }

        for s in samples {
            tx.prepare_cached(
                "INSERT INTO samples(collector, metric, time_ms, time_off, value, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(collector, metric, time_ms) DO UPDATE SET
                    value = excluded.value, data = excluded.data",
            )?
            .execute(params![
                s.collector,
                s.metric,
                s.time.utc_ms,
                s.time.offset_min,
                s.value,
                s.data
            ])?;
            report.samples_written += 1;
            // A sample describes up to MAX_SAMPLE_WINDOW_MS before its time.
            let start = s.time.plus_ms(-MAX_SAMPLE_WINDOW_MS);
            touch_days(&mut touched, s.collector, start, s.time.utc_ms + 1);
        }

        for it in items {
            let n = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO items(collector, id, time_ms, time_off, attrs)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )?
                .execute(params![
                    it.collector,
                    it.id,
                    it.time.utc_ms,
                    it.offset_known.then_some(it.time.offset_min),
                    it.attrs
                ])?;
            if n > 0 {
                report.items_written += 1;
                touched.insert((it.collector.to_string(), it.time.local_date()));
            }
        }

        for (collector, u) in scans {
            for f in &u.files_put {
                report.scan_rows_changed += tx
                    .prepare_cached(
                        "INSERT INTO scan_files(collector, path_hash, size, mtime_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(collector, path_hash) DO UPDATE SET
                            size = excluded.size, mtime_ms = excluded.mtime_ms",
                    )?
                    .execute(params![
                        collector,
                        f.path_hash as i64,
                        f.size as i64,
                        f.mtime_ms
                    ])?;
            }
            for h in &u.files_del {
                report.scan_rows_changed += tx
                    .prepare_cached(
                        "DELETE FROM scan_files WHERE collector = ?1 AND path_hash = ?2",
                    )?
                    .execute(params![collector, *h as i64])?;
            }
            if let Some(state) = &u.state {
                report.scan_rows_changed += tx.execute(
                    "INSERT INTO scan_state(collector, state) VALUES (?1, ?2)
                     ON CONFLICT(collector) DO UPDATE SET state = excluded.state
                     WHERE state IS NOT excluded.state",
                    params![collector, state],
                )?;
            }
            if let Some(done) = u.done_at {
                report.scan_rows_changed += tx.execute(
                    "INSERT INTO scan_state(collector, last_scanned_ms) VALUES (?1, ?2)
                     ON CONFLICT(collector) DO UPDATE SET last_scanned_ms = excluded.last_scanned_ms",
                    params![collector, done.utc_ms],
                )?;
            }
        }

        if max_session > self.max_session_ms {
            tx.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![KEY_MAX_SESSION, max_session.to_string()],
            )?;
        }

        for (collector, date) in &touched {
            report.metric_rows_changed +=
                recompute_day(&tx, collector, *date, max_session, rollup)?;
            report.days_recomputed += 1;
        }
        tx.commit()?;
        self.max_session_ms = max_session;
        Ok(report)
    }

    /// Recompute one collector's metrics for one local day (e.g. after a
    /// rollup definition changes). Returns the number of rows changed.
    pub fn recompute_day(
        &mut self,
        collector: &str,
        date: NaiveDate,
        rollup: &RollupFn<'_>,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let n = recompute_day(&tx, collector, date, self.max_session_ms, rollup)?;
        tx.commit()?;
        Ok(n)
    }

    // ---------- reads ----------

    pub fn daily_metrics(&self, date: NaiveDate) -> Result<Vec<MetricRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, collector, metric, key, value FROM daily_metrics
             WHERE date = ?1 ORDER BY collector, metric, value DESC, key",
        )?;
        let rows = stmt
            .query_map([date.to_string()], metric_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    pub fn all_daily_metrics(&self) -> Result<Vec<MetricRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT date, collector, metric, key, value FROM daily_metrics
             ORDER BY date, collector, metric, key",
        )?;
        let rows = stmt
            .query_map([], metric_row)?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Sessions of `collector` (all collectors if `None`) overlapping
    /// `[from_ms, to_ms)` UTC, ordered by start.
    pub fn sessions(
        &self,
        collector: Option<&str>,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT collector, key, start_ms, start_off, end_ms, end_off, open FROM sessions
             WHERE (?1 IS NULL OR collector = ?1) AND start_ms < ?3 AND end_ms > ?2
             ORDER BY start_ms, collector",
        )?;
        let rows = stmt
            .query_map(params![collector, from_ms, to_ms], |r| {
                Ok(SessionRow {
                    collector: r.get(0)?,
                    key: r.get(1)?,
                    start: Timestamp::new(r.get(2)?, r.get(3)?),
                    end: Timestamp::new(r.get(4)?, r.get(5)?),
                    open: r.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
}

fn metric_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MetricRow> {
    Ok(MetricRow {
        date: r.get(0)?,
        collector: r.get(1)?,
        metric: r.get(2)?,
        key: r.get(3)?,
        value: r.get(4)?,
    })
}

fn upsert_session(
    tx: &Transaction<'_>,
    collector: &str,
    key: &str,
    start: Timestamp,
    end: Timestamp,
    open: bool,
) -> Result<()> {
    tx.execute(
        "INSERT INTO sessions(collector, start_ms, start_off, end_ms, end_off, key, open)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(collector, start_ms) DO UPDATE SET
            start_off = excluded.start_off, end_ms = excluded.end_ms,
            end_off = excluded.end_off, key = excluded.key, open = excluded.open",
        params![
            collector,
            start.utc_ms,
            start.offset_min,
            end.utc_ms,
            end.offset_min,
            key,
            open as i64
        ],
    )?;
    Ok(())
}

/// Mark every local day (by the session's start offset) from `start` to `end_ms`.
fn touch_days(
    set: &mut BTreeSet<(String, NaiveDate)>,
    collector: &str,
    start: Timestamp,
    end_ms: i64,
) {
    let off = start.local_ms() - start.utc_ms;
    let first = date_from_local_ms(start.local_ms());
    // An exclusive end exactly at midnight does not touch the next day.
    let last = date_from_local_ms((end_ms + off - 1).max(start.local_ms()));
    let mut d = first;
    while d <= last {
        set.insert((collector.to_string(), d));
        match d.succ_opt() {
            Some(n) => d = n,
            None => break,
        }
    }
}

/// Session slices of `collector` that fall in local day `date`, sorted by start.
fn day_slices(
    tx: &Transaction<'_>,
    collector: &str,
    date: NaiveDate,
    max_session_ms: i64,
) -> Result<Vec<Slice>> {
    let day0 = local_midnight_ms(date);
    let day1 = day0 + MS_PER_DAY;
    let mut stmt = tx.prepare_cached(
        "SELECT key, start_ms, start_off, end_ms FROM sessions
         WHERE collector = ?1 AND start_ms >= ?2 AND start_ms < ?3 AND end_ms > ?4
         ORDER BY start_ms",
    )?;
    let lower = day0 - OFFSET_MARGIN_MS - max_session_ms - 1;
    let rows = stmt.query_map(
        params![
            collector,
            lower,
            day1 + OFFSET_MARGIN_MS,
            day0 - OFFSET_MARGIN_MS
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i32>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (key, start_ms, off, end_ms) = row?;
        let off_ms = i64::from(off) * crate::time::MS_PER_MIN;
        let (ls, le) = (start_ms + off_ms, end_ms + off_ms);
        let (cs, ce) = (ls.max(day0), le.min(day1));
        if cs >= ce {
            continue;
        }
        out.push(Slice {
            key,
            start: Timestamp::new(cs - off_ms, off),
            end: Timestamp::new(ce - off_ms, off),
            starts_session: ls >= day0,
        });
    }
    out.sort_by_key(|s| s.start.utc_ms);
    Ok(out)
}

/// Items of `collector` whose local date is `date`, sorted by time.
fn day_items(tx: &Transaction<'_>, collector: &str, date: NaiveDate) -> Result<Vec<DayItem>> {
    let day0 = local_midnight_ms(date);
    let mut stmt = tx.prepare_cached(
        "SELECT id, time_ms, time_off, attrs FROM items
         WHERE collector = ?1 AND time_ms >= ?2 AND time_ms < ?3 ORDER BY time_ms",
    )?;
    let rows = stmt.query_map(
        params![
            collector,
            day0 - OFFSET_MARGIN_MS,
            day0 + MS_PER_DAY + OFFSET_MARGIN_MS
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i32>>(2)?,
                r.get::<_, String>(3)?,
            ))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let (id, ms, off, attrs) = row?;
        let time = Timestamp::new(ms, off.unwrap_or(0));
        if time.local_date() != date {
            continue;
        }
        out.push(DayItem {
            id,
            time,
            attrs: serde_json::from_str(&attrs).unwrap_or(serde_json::Value::Null),
        });
    }
    Ok(out)
}

/// Samples of `collector` whose local time is on `date` or up to
/// [`MAX_SAMPLE_WINDOW_MS`] after it, sorted by time then metric.
///
/// Looked up metric by metric (the primary key is (collector, metric,
/// time)), so a day's rollup reads only that day's rows however much history
/// is stored.
fn day_samples(tx: &Transaction<'_>, collector: &str, date: NaiveDate) -> Result<Vec<DaySample>> {
    let day0 = local_midnight_ms(date);
    let day1 = day0 + MS_PER_DAY;
    let mut metrics: Vec<String> = Vec::new();
    {
        let mut next = tx.prepare_cached(
            "SELECT min(metric) FROM samples WHERE collector = ?1 AND metric > ?2",
        )?;
        let mut after = String::new();
        while let Some(m) =
            next.query_row(params![collector, after], |r| r.get::<_, Option<String>>(0))?
        {
            after.clone_from(&m);
            metrics.push(m);
        }
    }
    let mut stmt = tx.prepare_cached(
        "SELECT time_ms, time_off, value, data FROM samples
         WHERE collector = ?1 AND metric = ?2 AND time_ms >= ?3 AND time_ms < ?4",
    )?;
    let mut out = Vec::new();
    for metric in metrics {
        let rows = stmt.query_map(
            params![
                collector,
                metric,
                day0 - OFFSET_MARGIN_MS,
                day1 + MAX_SAMPLE_WINDOW_MS + OFFSET_MARGIN_MS
            ],
            |r| {
                Ok((
                    Timestamp::new(r.get(0)?, r.get(1)?),
                    r.get::<_, f64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        for row in rows {
            let (time, value, data) = row?;
            let local = time.local_ms();
            if local < day0 || local >= day1 + MAX_SAMPLE_WINDOW_MS {
                continue;
            }
            out.push(DaySample {
                metric: metric.clone(),
                time,
                value,
                data: data
                    .and_then(|d| serde_json::from_str(&d).ok())
                    .unwrap_or(serde_json::Value::Null),
            });
        }
    }
    out.sort_by(|a, b| (a.time.utc_ms, &a.metric).cmp(&(b.time.utc_ms, &b.metric)));
    Ok(out)
}

/// Recompute and diff-write one day's metrics for one collector.
fn recompute_day(
    tx: &Transaction<'_>,
    collector: &str,
    date: NaiveDate,
    max_session_ms: i64,
    rollup: &RollupFn<'_>,
) -> Result<usize> {
    let slices = day_slices(tx, collector, date, max_session_ms)?;
    let items = day_items(tx, collector, date)?;
    let samples = day_samples(tx, collector, date)?;
    let fresh: BTreeMap<(String, String), f64> =
        if slices.is_empty() && items.is_empty() && samples.is_empty() {
            BTreeMap::new()
        } else {
            let input = DayInput {
                date,
                slices: &slices,
                items: &items,
                samples: &samples,
            };
            match rollup(collector, &input) {
                Some(rows) => rows
                    .into_iter()
                    .map(|m| ((m.metric.to_string(), m.key), m.value))
                    .collect(),
                None => return Ok(0), // unknown collector: leave its rows alone
            }
        };
    let date_s = date.to_string();
    let existing: BTreeMap<(String, String), f64> = {
        let mut stmt = tx.prepare_cached(
            "SELECT metric, key, value FROM daily_metrics WHERE date = ?1 AND collector = ?2",
        )?;
        stmt.query_map(params![date_s, collector], |r| {
            Ok(((r.get(0)?, r.get(1)?), r.get(2)?))
        })?
        .collect::<std::result::Result<_, _>>()?
    };
    let mut changed = 0;
    for mk in existing.keys() {
        if !fresh.contains_key(mk) {
            tx.execute(
                "DELETE FROM daily_metrics WHERE date = ?1 AND collector = ?2 AND metric = ?3 AND key = ?4",
                params![date_s, collector, mk.0, mk.1],
            )?;
            changed += 1;
        }
    }
    for (mk, v) in &fresh {
        if existing.get(mk).is_some_and(|old| (old - v).abs() < 1e-6) {
            continue;
        }
        tx.execute(
            "INSERT INTO daily_metrics(date, collector, metric, key, value) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(date, collector, metric, key) DO UPDATE SET value = excluded.value",
            params![date_s, collector, mk.0, mk.1, v],
        )?;
        changed += 1;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_from_empty_database() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.schema_version().unwrap(), migrations::MIGRATIONS.len());
        for t in [
            "sessions",
            "samples",
            "daily_metrics",
            "scan_state",
            "settings",
        ] {
            let n: i64 = s
                .conn()
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "table {t}");
        }
    }

    #[test]
    fn reopen_is_idempotent_and_keeps_data() {
        let dir = std::env::temp_dir().join(format!("wrapped-test-{}", std::process::id()));
        let path = dir.join("reopen.db");
        let _ = std::fs::remove_file(&path);
        {
            let s = Store::open(&path).unwrap();
            s.set_setting("idle_threshold_secs", "240").unwrap();
            let mode: String = s
                .conn()
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.load_settings().unwrap().0.idle_threshold_secs, 240);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
        assert!(matches!(
            migrations::migrate(&mut conn),
            Err(Error::NewerSchema { found: 999, .. })
        ));
    }

    #[test]
    fn set_setting_validates() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.set_setting("idle_threshold_secs", "1").is_err());
        assert_eq!(
            s.set_setting("exclude.apps", r#"["a","a"]"#).unwrap(),
            r#"["a"]"#
        );
    }
}

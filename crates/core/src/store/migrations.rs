//! Embedded, ordered schema migrations. `PRAGMA user_version` holds the number
//! of migrations applied. Rules: append only; never edit or reorder a shipped
//! migration; never DROP or rewrite user data (add columns/tables instead).

use rusqlite::Connection;

use super::Error;

/// `MIGRATIONS[i]` takes the schema from version `i` to `i + 1`.
pub const MIGRATIONS: &[&str] = &[
    // v1: initial schema (DESIGN.md §8).
    //
    // Tables are WITHOUT ROWID and clustered on their natural key so that a
    // flush touches as few 4 KB pages as possible: new sessions for a
    // collector append to one leaf page, and a day's metrics sit together.
    r#"
    CREATE TABLE sessions (
        collector  TEXT    NOT NULL,
        start_ms   INTEGER NOT NULL,           -- UTC Unix ms
        start_off  INTEGER NOT NULL,           -- local UTC offset, minutes east
        end_ms     INTEGER NOT NULL,
        end_off    INTEGER NOT NULL,
        key        TEXT    NOT NULL,           -- e.g. bundle id, away reason
        open       INTEGER NOT NULL DEFAULT 0, -- 1 = still running at last flush
        PRIMARY KEY (collector, start_ms)
    ) WITHOUT ROWID;

    CREATE TABLE samples (
        collector  TEXT    NOT NULL,
        metric     TEXT    NOT NULL,
        time_ms    INTEGER NOT NULL,
        time_off   INTEGER NOT NULL,
        value      REAL    NOT NULL,
        PRIMARY KEY (collector, metric, time_ms)
    ) WITHOUT ROWID;

    CREATE TABLE daily_metrics (
        date       TEXT    NOT NULL,           -- local date, YYYY-MM-DD
        collector  TEXT    NOT NULL,
        metric     TEXT    NOT NULL,
        key        TEXT    NOT NULL DEFAULT '',
        value      REAL    NOT NULL,
        PRIMARY KEY (date, collector, metric, key)
    ) WITHOUT ROWID;

    CREATE TABLE scan_state (
        collector       TEXT PRIMARY KEY,
        last_scanned_ms INTEGER,
        state           TEXT                   -- collector-defined JSON
    ) WITHOUT ROWID;

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL                    -- JSON
    ) WITHOUT ROWID;
    "#,
    // v2: scanned collectors (Milestone 2).
    //
    // `items`: one row per discrete thing a scanned collector found (a git
    // commit, a photo). Neither a session (no duration) nor a sample (not a
    // periodic counter, needs a dedupe id and a few attributes). Inserted with
    // INSERT OR IGNORE on (collector, id), so re-reading the same commit or shot
    // from another clone / copy never counts twice. `time_off` is NULL when the
    // source has no UTC offset (camera time as written); `time_ms` then holds
    // that naive local time as if it were UTC.
    //
    // `scan_files`: per-file incremental cache (photos). The key is a salted
    // 64-bit hash of the path; no path or file name is stored. Kept out of
    // `scan_state.state` so a daily scan that finds nothing new writes nothing
    // (a JSON blob would be rewritten whole).
    r#"
    CREATE TABLE items (
        collector  TEXT    NOT NULL,
        id         TEXT    NOT NULL,           -- dedupe key (commit hash, shot hash)
        time_ms    INTEGER NOT NULL,           -- UTC Unix ms (naive local if time_off IS NULL)
        time_off   INTEGER,                    -- local UTC offset in minutes; NULL = unknown
        attrs      TEXT    NOT NULL,           -- JSON object, collector-defined
        PRIMARY KEY (collector, id)
    ) WITHOUT ROWID;
    CREATE INDEX items_by_time ON items(collector, time_ms);

    CREATE TABLE scan_files (
        collector  TEXT    NOT NULL,
        path_hash  INTEGER NOT NULL,           -- salted SipHash-2-4 of the path
        size       INTEGER NOT NULL,
        mtime_ms   INTEGER NOT NULL,
        PRIMARY KEY (collector, path_hash)
    ) WITHOUT ROWID;
    "#,
    // v3: several values per sample row (Milestone 3).
    //
    // The Disk & Memory collector records seven values per reading. As seven
    // `samples` rows they land in seven separate parts of the (collector,
    // metric, time) B-tree, so every flush dirtied ~7 extra 4 KB pages
    // (measured 3.9 MB/day for 16 h awake against a 1-2 MB budget). One row
    // per reading, the other values in `data`, appends to a single leaf page.
    // NULL for ordinary single-value samples.
    r#"
    ALTER TABLE samples ADD COLUMN data TEXT;  -- JSON object, collector-defined
    "#,
    // v4: AI usage (Milestone 6).
    //
    // `ai_usage`: token counts summed per (UTC hour, tool, session, model,
    // project). One row per request would be ~50 MB a year for Claude Code
    // alone (about 550 requests a day here) and a random insert each; the
    // hourly sums are a few thousand rows. Time first in the key, so new data
    // appends. `off` is the local UTC offset at the bucket's first request
    // (the day a bucket belongs to is decided by local time). `project` is the
    // last path component only. Never prompt or response text, never a cost.
    //
    // `ai_seen`: one 64-bit hash per counted request (tool + the tool's own
    // request id), the dedupe set that makes rescans, resumed / forked
    // sessions and repeated log lines count once. Keyed by the hash alone: one
    // request is written as several lines whose timestamps differ by up to
    // minutes (measured: up to 172 s), so time cannot be part of the key. Kept
    // forever, so a log restored from a backup months later still counts once.
    //
    // `scan_files` gets the byte offset after the last complete line read
    // (`pos`) and a hash of the file's first bytes (`head`): an unchanged
    // prefix means a log only grew, so the next scan reads from `pos`.
    r#"
    CREATE TABLE ai_usage (
        hour_ms     INTEGER NOT NULL,
        tool        TEXT    NOT NULL,
        session     TEXT    NOT NULL,
        model       TEXT    NOT NULL,
        project     TEXT    NOT NULL,
        off         INTEGER NOT NULL,
        input       INTEGER NOT NULL DEFAULT 0,
        output      INTEGER NOT NULL DEFAULT 0,
        cache_read  INTEGER NOT NULL DEFAULT 0,
        cache_write INTEGER NOT NULL DEFAULT 0,
        requests    INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (hour_ms, tool, session, model, project)
    ) WITHOUT ROWID;

    CREATE TABLE ai_seen (
        id_hash INTEGER PRIMARY KEY
    ) WITHOUT ROWID;

    ALTER TABLE scan_files ADD COLUMN pos  INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE scan_files ADD COLUMN head INTEGER NOT NULL DEFAULT 0;
    "#,
];

pub fn schema_version(conn: &Connection) -> Result<usize, Error> {
    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(v.max(0) as usize)
}

/// Apply every pending migration, each in its own transaction.
pub fn migrate(conn: &mut Connection) -> Result<usize, Error> {
    let current = schema_version(conn)?;
    if current > MIGRATIONS.len() {
        return Err(Error::NewerSchema {
            found: current,
            supported: MIGRATIONS.len(),
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(MIGRATIONS.len())
}

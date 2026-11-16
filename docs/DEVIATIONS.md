# Deviations from DESIGN.md

Accepted differences between the implementation and `docs/DESIGN.md`. The
design doc itself is left as written.

## Collector trait (§6)

- `on_event(&mut self, ev: &SystemEvent)` added. Live collectors do not
  subscribe to the OS themselves: the platform layer turns OS signals into
  plain `SystemEvent`s and the engine feeds them to every running collector,
  so collectors are pure state machines testable without macOS.
- `rollup(&self, day: &DayInput) -> Vec<MetricValue>` added, so the framework
  can recompute `daily_metrics` for any collector without knowing what it
  records.
- `scan` takes `since: Option<Timestamp>` (`None` = never scanned) instead of
  `Time`. It runs on a background thread, on a fresh instance returned by the
  added `scanner(&self) -> Option<Box<dyn Collector>>`, so the registered
  instance keeps serving `rollup` while a scan is running.
- `enable_defaults(&self, &Settings)` added: settings a collector stores
  explicitly when turned on (git: `git.author_emails` from
  `git config --global user.email`).
- `DayInput` carries the day's `items` next to the session `slices`.

## Data model (§8)

- `sessions` has extra columns: `start_off` and `end_off` (local UTC offset in
  minutes at that moment) and `open` (1 while the session was still running at
  the last flush; cleared on startup by crash recovery).
- `samples` has `time_off` (local UTC offset of the sample).
- `scan_state` has a `state` column (collector-defined JSON) next to
  `last_scanned_ms`.
- `daily_metrics.date` is the local date as text `YYYY-MM-DD`.
- `items(collector, id, time_ms, time_off, attrs)` added (migration v2) for
  scanned collectors: one row per commit or photo. These fit neither
  `sessions` (no duration, and the primary key is a start time, which two
  photos or commits can share) nor `samples` (no dedupe id, one numeric value,
  no attributes). Primary key `(collector, id)` with `INSERT OR IGNORE` makes
  re-reading idempotent (same commit in two clones or rebased, RAW+JPEG pair, moved
  file); an index on `(collector, time_ms)` serves the daily rollup.
  `time_off` is NULL when the source had no UTC offset (photo without
  OffsetTimeOriginal); `time_ms` then holds the camera's naive local time.
  Append-only like the other tables: never rewritten, only deleted on request.
- `scan_files(collector, path_hash, size, mtime_ms)` added (migration v2):
  the photos per-file cache, keyed by a salted 64-bit SipHash of the path (no
  path or name stored; the salt lives in the collector's `scan_state`). A
  table rather than a JSON blob in `scan_state.state` so an incremental scan
  that finds nothing new writes nothing (a blob for ~10^4–10^5 files would be
  rewritten whole every day, blowing the 1–2 MB/day write budget).
- `scan_state.last_scanned_ms` is the *start* of the last completed scan; a
  cancelled scan does not update it (its progress is in `state` /
  `scan_files`).
- Pending on-demand scan requests from the CLI to a running collector are
  `internal.scan_request.<collector>` rows in `settings`.
- `settings` holds one bookkeeping row, `internal.max_session_ms` (longest
  session ever stored), which bounds how far back a day's rollup has to look.
  Keys starting with `internal.` are not user settings.

## Active / idle collector (§7)

- Away periods include sleep. Each period is keyed by the reason it started
  with: `idle`, `sleep`, `display_off`, `locked`, `screensaver`. A period that
  starts idle and continues into sleep stays one period keyed `idle`. Pause and
  power-off record nothing.
- Extra metric `active_idle/away_secs_by_reason` (key = reason).

## Git and photos collectors (§7)

- Git counts commits by **author** email and author time; merges count. Refs
  read: `refs/heads`, `refs/tags`, `refs/remotes` (not stashes, notes or a
  detached HEAD).
- A git item's id is not the commit hash but a hash of (author email,
  author time in seconds, author offset, first line of the message), so a
  rebase (including `--force-rebase`), an amend that keeps the message, a
  cherry-pick and a second clone all count once. The message line only goes
  into the hash; it is never stored or logged. Limit: rewording a commit's
  first line makes it count again. Commit hashes are not stored as item data;
  the only ones kept are the ref tips in git's `scan_state` (the incremental
  watermark: the next scan excludes commits reachable from them).
- Photos: files with no capture date, no EXIF, or an unsupported format are
  skipped and reported (`skipped_no_date`, `skipped_no_exif`,
  `skipped_unsupported`). Burst shots in the same second with no
  SubSecTimeOriginal and no image number share an id and count once.
- Extra metric keys: `(unknown)` for a missing camera, lens or focal length,
  so per-key sums equal `shots`.

## Metric definitions

- `app_usage/app_switches`: number of sessions that start exactly where the
  previous session (on the same local day) ended, with a different app. A gap
  (idle, lock, an excluded app) between two apps is not a switch, and neither
  is the after-midnight continuation of a session or a split caused by a UTC
  offset change.
- `active_idle/away_periods`: number of away periods that start on that local
  day. A period crossing midnight counts once, on the day it started; a split
  caused by a UTC offset change does not start a new period.

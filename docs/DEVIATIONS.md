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
- `DayInput` carries the day's `items` next to the session `slices`, and
  (Milestone 3) the `samples` whose time is on the day or up to one hour
  after it (`MAX_SAMPLE_WINDOW_MS`: a sample describes at most the hour
  before it); the collector decides which part of each belongs to the day.
- `Collector::on_data_deleted()` added: called when a running collector's
  data is deleted, so it can drop state that would carry pre-delete time
  (disk_memory re-baselines its counters).
- System counters reach collectors through the `Sink`
  (`Sink::read_disk_memory()`, backed by a `SystemProbe` the platform layer
  gives the engine), not through events: the collector decides when to read.
  `Sink::system_asleep()` exposes the engine's asleep state (from
  `WillSleep` until `DidWake` or input after it), set before each event is
  dispatched.

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
- `samples.data` (migration v3): a JSON object with further values of the
  same reading, NULL for single-value samples. Disk & Memory stores one row
  per reading (metric `reading`, value = awake seconds covered, the deltas,
  compressed memory and pressure level in `data`) instead of one row per
  value: seven rows per reading sat in seven parts of the
  `(collector, metric, time)` B-tree and cost ~7 extra pages per flush
  (3.87 MB/day measured for a 16 h day, against 1.60 MB as one row). See
  MILESTONE3.md.
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
- `Collector::scan_fingerprint(&Settings)` added, stored as
  `internal.scan_fingerprint.<collector>` in `settings`. When it changes (git:
  the author email list), the collector's items, scan state and daily metrics
  are deleted so the next scan re-reads everything under the new rule; done
  by `settings set` when no collector runs, otherwise by the running
  collector at its next tick (and by `scan` on startup).
- `settings` holds one bookkeeping row, `internal.max_session_ms` (longest
  session ever stored), which bounds how far back a day's rollup has to look.
  Keys starting with `internal.` are not user settings.

## Active / idle collector (§7)

- Away periods include sleep. Each period is keyed by the reason it started
  with: `idle`, `sleep`, `display_off`, `locked`, `screensaver`. A period that
  starts idle and continues into sleep stays one period keyed `idle`. Pause and
  power-off record nothing.
- Extra metric `active_idle/away_secs_by_reason` (key = reason).
- Display assertions (§13: "an app holding a keep display awake request …
  counts as active"): only an assertion held by the **frontmost** app's
  process, or created on its behalf (`AssertionOnBehalfOfPID`), counts.
  Assertions held by background processes (`caffeinate`, a terminal's child
  job, daemons) never keep the user present; idle is backdated to the last
  input as if there were none. While the front app holds one there is no
  timeout; after it is released, apps on the active-without-input list get
  their timeout from the last tick that saw it.

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

## Disk & Memory collector (§7)

- "Every 5 min" = the first minute tick at least 4.5 min after the previous
  reading, while awake and not paused; dark-wake ticks never read.
- A reading after sleep records one delta (everything since the previous
  reading) at the wake tick; it covers only the awake seconds (before the
  sleep plus since the wake), so its bytes land just before the wake tick,
  not spread over the night.
- Counter resets (DESIGN.md §8) are detected by boot time first (more than
  1 s apart), then by any counter going down or a rate above 32 GiB/s; any of
  them re-baselines with no sample for that interval.
- Pause drops the baseline: nothing done while paused is recorded.
- "Disk GB read/written" is internal storage only (disk images, external
  drives and the SD card reader excluded); "compressed memory" is the
  physical memory the compressor occupies.
- Extra metric `disk_memory/bytes_written_by_hour` (key `00`..`23`);
  memory pressure is `pressure_secs_by_level` (key `normal|warning|critical`),
  compressed memory `compressed_peak_bytes` and `compressed_avg_bytes`
  (time-weighted).
- Write budget (accepted, 2026-11-17): with Disk & Memory on, a 16 h day
  writes 1.60 MB and a 24 h awake day 2.64 MB, over the ~2 MB/day guideline
  (1.58 MB of that is the M1 collectors alone). Accepted for now: the
  collector is off by default, 24 h awake is rare on a laptop, and ~1 GB/year
  is negligible for an SSD. Revisit at Milestone 9 (an always-on Windows
  desktop may hit the 24 h case daily). The known lever is holding
  disk_memory samples in memory across several flushes (e.g. written every
  30 min), which risks up to ~30 min of disk samples on a crash, not activity
  data.

## Stats engine (§9, §11; Milestone 4)

- `MetricDef` has `agg` (`Sum`, `Max`, `Mean` of the days with a value) next
  to `unit` (`Seconds`, `Count`, `Bytes`), so ranges combine every metric
  without knowing the collector.
- Stats read `daily_metrics` for totals, series, top-N, comparisons and time
  of day; records that need exact times (longest session, latest night) read
  `sessions`. "Latest night" counts activity before 05:00 as the previous
  day's night; "longest session" joins pieces split only by a UTC offset
  change and belongs to the date it started.
- Percent change from a previous value of zero is reported as none (both
  zero: 0 %).
- `Store::open_read_only` for reading another database without migrating it.

## Metric definitions

- `app_usage/app_switches`: number of sessions that start exactly where the
  previous session (on the same local day) ended, with a different app. A gap
  (idle, lock, an excluded app) between two apps is not a switch, and neither
  is the after-midnight continuation of a session or a split caused by a UTC
  offset change.
- `active_idle/away_periods`: number of away periods that start on that local
  day. A period crossing midnight counts once, on the day it started; a split
  caused by a UTC offset change does not start a new period.

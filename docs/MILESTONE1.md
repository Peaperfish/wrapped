# Milestone 1: collector

Collector + plugin system + SQLite + daily metrics + tracking settings + tray +
launch at login. macOS only; all platform code is behind a platform module so
the Windows port (Milestone 9) does not touch core logic.

Accepted differences from the design doc are listed in
[DEVIATIONS.md](DEVIATIONS.md).

## What was built

### `crates/core` (no platform code, builds and tests on macOS and Windows)

- **Collector plugin trait** (`collector.rs`): `info`, `kind`, `start(Sink)`,
  `on_event`, `scan`, `stop`, `metrics`, `rollup`. Collectors never write to
  disk; they declare sessions and samples through a `Sink` into an in-memory
  buffer. Adding a collector = one file in `collectors/` + one entry in the
  `register!` line.
- **System event bus** (`event.rs`): `FrontApp`, `IdleTick`, `WillSleep`,
  `DidWake`, `ScreensSlept`, `ScreensWoke`, `ScreenLocked`, `ScreenUnlocked`,
  `WillPowerOff`, `Paused`, `Resumed`.
- **Presence state machine** (`presence.rs`), shared by both collectors: idle
  threshold, active-without-input apps, display-sleep assertions, sleep,
  display sleep, lock, lock screen / screensaver as away, power-off.
- **Collectors**: `app_usage` (front-app sessions) and `active_idle` (away
  periods keyed `idle`, `sleep`, `display_off`, `locked`, `screensaver`). Both on
  by default.
- **Time**: every timestamp is UTC ms + the local UTC offset at that moment.
  Durations come from wall-clock differences, never tick counts. A session's
  day is decided by its local time; sessions crossing midnight are split for
  daily metrics; a UTC offset change splits a session within a minute.
- **SQLite** (`store/`): `sessions`, `samples`, `daily_metrics`, `scan_state`,
  `settings`; `PRAGMA user_version` with embedded append-only migrations;
  WAL + `synchronous=NORMAL`; WITHOUT ROWID tables clustered on their keys.
- **Shared batched writer**: one transaction every ~10 min and immediately on
  `WillSleep`, `WillPowerOff`, quit, SIGTERM and SIGINT. Each flush also
  upserts the open session with `end = flush time`, marked open; startup
  closes any session still marked open (a crash loses at most ~10 min).
- **Daily metrics**, recomputed on flush for the local days touched, writing
  only rows whose value changed: `app_usage/screen_time_secs`, `app_secs`
  (key = bundle id), `active_secs_by_hour` (key `00`..`23`), `app_switches`;
  `active_idle/away_secs`, `away_periods`, `away_secs_by_reason`.
- **Settings**: enabled collectors, `idle_threshold_secs` (180),
  `active_without_input.apps` and `.timeout_secs` (1800), `exclude.apps`,
  `pause` (off / until a time / indefinite). `disable_collector(keep_data)` and
  `delete_collector_data()` for the future keep-or-delete dialog.
- **Export**: streaming JSON (one file) and CSV (one file per table).

### `crates/collector` (the binary)

- **macOS platform layer** (`src/platform/macos/`), ported from the M0 spike:
  NSWorkspace front-app, sleep/wake, display sleep/wake and power-off
  notifications; `com.apple.screenIsLocked` / `screenIsUnlocked`; a run-loop
  `NSTimer` every 60 s (5 s tolerance) reading
  `CGEventSourceSecondsSinceLastEventType` and `IOPMCopyAssertionsByProcess`.
  No Accessibility, Input Monitoring or Screen Recording permission; no event
  taps or keyboard hooks. Every Objective-C callback body runs in an
  autoreleasepool.
- **Tray**: native `NSStatusItem` menu (status line, Pause for 1 Hour / Until
  Tomorrow / Indefinitely, Resume, Open Data Folder, Quit). Accessory
  activation policy plus `LSUIElement`: no Dock icon.
- **Shutdown**: Quit, SIGTERM, SIGINT and logout all go through
  `applicationWillTerminate:`, which stops the collectors and flushes.
  SIGUSR1 flushes immediately. A `flock` on `wrapped.lock` prevents two
  collectors on one database.
- **Launch at login**: `SMAppService.mainApp` (macOS 13+) from a minimal
  ad-hoc-signed `Wrapped.app` built by `scripts/macos/bundle.sh`.
- **Temporary CLI** (see `wrapped-collector help`): `settings get|set`,
  `stats [today|YYYY-MM-DD]`, `sessions [today|YYYY-MM-DD]`,
  `export --json <file>` / `--csv <dir>`, `flush`, `login enable|disable|status`.
- **Debug logging**: `--debug` or `WRAPPED_DEBUG=1` logs every system event,
  the wall-clock gap between consecutive idle ticks, and every flush (rows
  written, kernel-counted bytes written by the process, WAL size) to stderr.
  A tick gap over 90 s with no sleep in between is always logged as a warning.

## Running it

```sh
cargo build --release -p wrapped-collector
./target/release/wrapped-collector --debug          # tray icon; Ctrl+C quits with a flush
./target/release/wrapped-collector stats today
./target/release/wrapped-collector sessions today
```

Database: `~/Library/Application Support/Wrapped/wrapped.db`
(`WRAPPED_DB=path` to use another file).

### Install with launch at login

```sh
scripts/macos/install.sh
~/Applications/Wrapped.app/Contents/MacOS/wrapped-collector login enable
```

`install.sh` builds the release bundle, quits a running installed collector
(SIGTERM, so it flushes), swaps `~/Applications/Wrapped.app` in place (same path
and bundle id, so the login item stays valid), relaunches it, checks that
`wrapped-collector --version` matches HEAD, and deletes every other
`Wrapped.app` under `target/` (unregistered from LaunchServices first). It
refuses to replace a collector running from elsewhere (a debug or measurement
run) unless given `--force`. Version: `CFBundleShortVersionString` is the crate
version, `CFBundleVersion` the short git hash (`-dirty` with uncommitted
changes); `wrapped-collector --version` prints both. Run `login enable` once
after the first install:

```sh
~/Applications/Wrapped.app/Contents/MacOS/wrapped-collector login enable
```

`login enable` should print `enabled`; if it says it requires approval, turn
Wrapped on in System Settings → General → Login Items. macOS shows a one-time
"Background Items Added" notice.

### Uninstall

```sh
~/Applications/Wrapped.app/Contents/MacOS/wrapped-collector login disable
osascript -e 'quit app "Wrapped"'       # or tray → Quit Wrapped
rm -rf ~/Applications/Wrapped.app
rm -rf ~/Library/Application\ Support/Wrapped   # only to delete all recorded data
```

## Decisions

- **Platform code is a module, not a crate.** Only the binary uses it; the
  boundary that matters (core never depends on an OS) is enforced by core
  being its own crate.
- **Idle backdating.** Normal apps: when idle is detected (on the minute
  tick), the session ends at the last evidence of use: the latest of last
  input, an app switch, wake / unlock / resume, or a tick where a display
  assertion was held. Apps on the active-without-input list: the session ends
  at last evidence of use + that list's timeout.
- **App switch, wake, unlock and Resume count as use.** If nobody is actually
  there, the next tick backdates to that moment and the zero-length session is
  dropped.
- **Defensive rules** for missed notifications: input made after sleep /
  display-off / power-off began clears that state; a gap of more than 5 min
  between ticks with no sleep notification is treated as sleep.
- **One open session per collector**, declared through
  `Sink::set_session(key, at)`; a UTC offset change splits it; rollups use the
  session's start offset.
- **WAL + `synchronous=NORMAL`**, 4 KB pages, WITHOUT ROWID tables and
  diff-only metric writes, to stay within the 1–2 MB/day write budget.
  Simulated 16-hour day (`cargo test -p wrapped-core --release --test
  write_budget -- --ignored --nocapture`): WAL 1,017,672 bytes + checkpoint
  36,864 bytes = 1,054,536 bytes; the kernel's per-process counter for the
  test process was 1,069,056 bytes. A simulation, not a field measurement.
- **SMAppService over a LaunchAgent**: Apple's current mechanism, shows in
  Login Items, no plist in `~/Library/LaunchAgents`. Needs the .app bundle.
- **Settings are re-read from the database on every tick**, so CLI changes
  apply within a minute without IPC.
- **Excluded apps** are recorded nowhere: no rows, and redacted in the debug log.
- **The collector ignores itself.** The platform layer passes its own app id
  (main bundle id, falling back to `io.github.peaperfish.wrapped`) to the
  engine. `FrontApp` events for it (e.g. opening the tray menu) are dropped:
  no session starts or ends, no app switch, no activity evidence, so the
  previous app's session continues. Other menu-bar apps are still recorded;
  exclude them in settings if unwanted.

## Known limitations

- **±60 s on return from idle.** Return is only seen at the next tick (or the
  next app switch / unlock / wake), placed at the last input that tick saw, so
  the first session after an idle period can start up to ~60 s late.
- **UTC offset changes** split sessions up to ~60 s after the change; daily
  attribution uses each session's start offset.
- **Crash loses up to ~10 min** (time since the last flush). A power cut can
  also lose the last committed flush(es): `synchronous=NORMAL` does not fsync
  every commit.
- **Any process holding `PreventUserIdleDisplaySleep` keeps the user active**,
  including background video or keep-awake utilities.
- **Timer stretching**: if App Nap or coalescing ever delayed a tick by more
  than 5 min with no sleep notification, that gap would be recorded as sleep.
  Not observed so far (gaps 59.9–65.0 s over 36 min); being measured in the
  overnight run.
- **Very short sessions** (a Cmd-Tab passing through apps) are stored and
  counted as switches; there is no minimum duration.
- **Away periods only exist while the collector runs**: time while the
  machine is shut down or the collector is not running is neither active nor away.
- **A cancelled shutdown** leaves tracking halted until the next input or app switch.
- **Manual wall-clock changes** can distort durations (negative ones are clamped to zero).
- **Changing a rollup definition** needs `Store::recompute_day` for past days;
  there is no CLI for it yet.
- **Per-flush disk bytes**: the kernel counter in the flush log usually shows
  +0, because writes stay in the page cache until a checkpoint or write-back;
  only long-run totals are meaningful.
- **`ps` RSS (~65–71 MB) is not the memory budget figure**: it includes
  shared AppKit framework pages. Activity Monitor's Memory column is the
  physical footprint (~11 MB).
- **The debug log is written by the collector**, so it adds to the process's
  bytes written; subtract its size when measuring disk writes.
- **Windows**: platform stub only (Milestone 9).

### Not verified live (covered by unit tests only, or not at all)

Tray Pause / Resume is now verified live (paused 16:09:34, resumed 16:10:12 on
2026-11-16; nothing recorded while paused).

- Display-only sleep (display off, system awake) via `pmset displaysleepnow`.
- Launch at login: `login enable` printed enabled and `Wrapped.app` started
  at 16:06:52 with the menu-bar icon and no Dock icon (2026-11-16); login after
  logout is not yet verified.
- Sleep via the Apple menu; power-off / logout flush path; behaviour on battery.
- Windows CI job with this code.

## Manual test results (Phase B, 2026-11-16, release build, real database)

| Test | Result |
|---|---|
| App switches | Each `FrontApp` became one session ending where the next began; metrics matched the rows (33 switches, 136 s). |
| 4 min idle | Session ended at the last evidence of use (15:27:23), not at the detecting tick (15:30:54); `idle` 15:27:23–15:33:17; resumed at the last input seen by the next tick. |
| `caffeinate -d -t 330` | Ticks at 193, 253, 313 s without input listed `caffeinate[pid]`, so the session stayed open; it ended at the last tick that saw the assertion (15:49:54). |
| Lock (Ctrl+Cmd+Q) | `ScreenLocked` + `loginwindow` ended the session; the Mac also slept; one `locked` period (3 m 03 s) until `ScreenUnlocked`; `loginwindow` never recorded as an app. `screensDidSleep` / `screensDidWake` fired (unconfirmed in M0). |
| `pmset displaysleepnow` | Skipped. |
| Lid close ~30 s | `WillSleep` flushed and ended the session; `sleep` 15:56:29–15:57:19. A tick fired 5 s after `WillSleep` reporting input from before it; correctly not treated as a return. |
| Tray Pause / Resume | Paused 16:09:34, resumed 16:10:12 (2026-11-16); no rows recorded while paused. |
| `stats today` | `screen_time_secs` 1482 = sum of `app_secs` = sum of hour buckets; `away_secs` 689 = idle 456 + locked 183 + sleep 49 over 4 periods. |
| Launch at login | `login enable` printed enabled; `Wrapped.app` started 16:06:52, menu-bar icon, no Dock icon. Logout/login not yet verified. |
| Resources after 36 min | Footprint 11.0 → 11.3 MB; RSS 70,976 → 65,584 KB; CPU 0:00.24; tick gaps 59.9–65.0 s without sleep, 0 warnings. |

## Overnight + workday verification

### Checklist

1. Start (from `~/code/wrapped`, with no other collector running):
   ```sh
   cargo build --release -p wrapped-collector
   mkdir -p ~/wrapped-phase-c
   nohup env WRAPPED_DEBUG=1 ./target/release/wrapped-collector >/dev/null 2>>~/wrapped-phase-c/debug.log &
   echo $! > ~/wrapped-phase-c/pid
   ```
2. At start, +1 h, +8 h (and the next morning, after the overnight sleep):
   ```sh
   scripts/macos/measure.sh | tee -a ~/wrapped-phase-c/measurements.txt
   ```
   (RSS via `ps -o rss=`, CPU via `ps -o time=`, footprint, file sizes, tick-gap stats.)
3. At the same times, in Activity Monitor (View → All Processes, search
   `wrapped-collector`):
   - Disk tab → **Bytes Written** (cumulative since start). Collector's own
     writes = Bytes Written − `debug_log` size from `measure.sh`.
   - Energy tab, **on battery**: **Energy Impact** and **Avg Energy Impact**.
4. Tick gaps: in `measure.sh` output, `gaps_over_90s` and `warnings` should
   stay 0, and `max_gap_s` should stay near 60–65. Any gap near 300 s with no
   sleep in between means the timer was stretched; stop and investigate.
   ```sh
   grep -E 'WARNING|tick gap' ~/wrapped-phase-c/debug.log | grep -v 'sleep/wake' | sort -k4 -n | tail -5
   ```
5. Sanity, after the workday:
   ```sh
   ./target/release/wrapped-collector stats today       # compare screen_time_secs with your estimate
   sqlite3 "$HOME/Library/Application Support/Wrapped/wrapped.db" "
     SELECT collector, key,
            datetime(start_ms/1000, 'unixepoch', 'localtime') AS start,
            datetime(end_ms/1000,   'unixepoch', 'localtime') AS end,
            (end_ms - start_ms) / 60000 AS minutes
     FROM sessions ORDER BY end_ms - start_ms DESC LIMIT 10;"
   ```
   The longest `app_usage` rows should be no longer than your real continuous
   use; overnight should appear as an `active_idle` `sleep` (or `idle` /
   `locked`) period, never as app time.
6. Stop with a final flush: `kill -TERM $(cat ~/wrapped-phase-c/pid)`.

### Results (to be filled in from the overnight run)

| Metric | Budget | Start | +1 h | +8 h | Next morning | Notes |
|---|---|---|---|---|---|---|
| RSS, `ps -o rss=` (KB) | — | | | | | |
| Physical footprint (MB) | < 20 MB | | | | | |
| Total CPU time, `ps -o time=` | ~0% | | | | | |
| `wrapped.db` size (bytes) | | | | | | |
| `wrapped.db-wal` size (bytes) | | | | | | |
| Debug log size (bytes) | | | | | | |
| Bytes Written, Activity Monitor | | | | | | |
| Collector writes (Bytes Written − debug log) | ~1–2 MB/day | | | | | |
| Energy Impact (battery) | ≈ 0 | | | | | |
| Avg Energy Impact (battery) | ≈ 0 | | | | | |
| Max tick gap without sleep (s) | < 300 | | | | | |
| Tick-gap warnings | 0 | | | | | |
| `screen_time_secs` vs own estimate | | | | | | |
| Longest `app_usage` session vs real use | | | | | | |

# Milestone 5: dashboard

The dashboard from DESIGN.md §9: a Tauri window over the stats engine
(Milestone 4) that runs only while it is open. Milestone 5 is split in two:

- **5a (this document's "built" part): the shell and the heatmap.** App,
  process model, data commands, header with range presets, the year
  heatmap, and a temporary plain-text summary of the selected range.
- **5b: the cards.** Screen time, top apps, categories, time of day, git,
  photos, disk & swap, memory pressure, records.

Accepted differences from the design doc are in [DEVIATIONS.md](DEVIATIONS.md).

## Plan (decisions)

- **Stack:** Tauri 2 + Svelte 5 + TypeScript + Vite. No chart library: the
  heatmap and the later charts are hand-drawn SVG. The Tauri CLI is an npm
  devDependency (no global installs). Lockfiles (`Cargo.lock`,
  `dashboard/ui/package-lock.json`) are committed.
- **Layout:** a new workspace member `dashboard/` (Rust, the Tauri backend,
  crate `wrapped-dashboard`) with the frontend in `dashboard/ui/`.
- **Data access:** the backend links `wrapped-core` and opens the database
  read-only (`Store::open_read_only`). It never writes. No read transaction
  stays open between commands (it would block the collector's WAL
  checkpoints). `--db <path>` picks the database; the default is the one the
  collector uses (`WRAPPED_DB`, else
  `~/Library/Application Support/Wrapped/wrapped.db`).
- **Commands** (same JSON shapes as `wrapped-collector stats range --json`
  where they overlap):
  - `meta()`: collectors and whether they are on, the earliest date with
    data, the last flush time, the heatmap metrics (only of collectors that
    are on), today's local date and the build id;
  - `summary(from, to, vsFrom?, vsTo?)`: the stats engine's result for the range (5b added the optional comparison range);
  - `series(year, metric)`: one value per day of the year, zeros included.

  TypeScript types are generated from the Rust structs with ts-rs into
  `dashboard/ui/src/lib/types/`; a test fails if they are stale.
- **Process model (macOS):** the dashboard is a nested app at
  `Wrapped.app/Contents/Helpers/Wrapped Dashboard.app`, bundle id
  `io.github.peaperfish.wrapped.dashboard`, a normal Dock app with the same
  version string as the collector (git hash, `-dirty` if uncommitted).
  Closing the window quits the process. The collector's tray menu has
  "Open Dashboard…", which runs `open` on the nested app (macOS brings a
  running instance to the front). **Windows:** the collector spawns the
  dashboard exe next to its own exe (compiles; not polished).
- **Install:** `scripts/macos/install.sh` builds the dashboard, places it in
  the bundle before the atomic replace, and verifies both versions; the
  graceful quit / relaunch of the collector is unchanged.
- **Frontend 5a:** dark theme as CSS variables (dark glass cards, one
  accent colour, tabular / monospace numbers); window 1200×820, minimum
  width 960. Header: Today · This week · This month · This year · Custom
  (Monday week start, local dates), the range label, "as of HH:MM" from the
  last flush, refresh. Heatmap: GitHub-style year grid, Monday-first rows,
  metric dropdown (default screen time), previous / next year bounded by the
  earliest data year and the current year; click, drag, shift-click; hover
  tooltip; selection and header range drive each other. Series fetched once
  per (year, metric); a range change only calls `summary`. Below it a
  temporary plain-text panel with the range and its totals. Empty states
  (no data, collector off) without errors.
- **CI:** Node with the npm cache, `npm ci`, svelte-check and the frontend
  build, then the dashboard crate in fmt, clippy `-D warnings` and tests on
  macOS and Windows.
- **Tests:** the three commands against a small fixture database built inside
  the test (never the real one), including `summary(from, to)` against the
  CLI's `stats range --json` for the same range.

## What was built (5a)

### Backend (`dashboard/`)

- `src/data.rs`: the commands as plain functions over a database path.
  Each call opens the database read-only, queries, and drops the connection,
  so nothing is held between calls and a collector turned off in the
  meantime applies to the next call. Opening costs well under a
  millisecond (see Performance).
- `src/lib.rs`: argument parsing (`--db`, `--debug`, `--version`), the
  Tauri commands `meta`, `summary`, `series` (async, off the main thread)
  and `ready`, and command timing logged to stderr with `--debug` /
  `WRAPPED_DEBUG=1`.
- The window starts hidden and is shown when the page reports `ready`
  (first frame with data rendered), so there is no blank flash; a 3 s
  fallback shows it anyway if the page never reports.
- `meta()` uses two new core reads, both index seeks only:
  `Store::earliest_metric_date(collectors)` (first `daily_metrics` date of
  a collector that is on) and `Store::last_recorded_ms(collectors)` (end of
  the newest session, an open session's end being the last flush; newest
  sample; latest scan start).
- `summary()` returns `Summary::to_json()` unchanged with
  `SUMMARY_TOP_N` (5 in 5a, 8 since 5b) keys per metric, now a core constant shared with the
  CLI.
- `series()` takes `"<collector>/<metric>"` (the `id` in
  `meta().metrics`); only metrics without keys of collectors that are on.
- `src/types.rs`: the Rust types and their TypeScript twins (ts-rs; integers
  map to `number`). `SummaryJson` describes `Summary::to_json()`; a test
  deserializes real output into it with `deny_unknown_fields` and compares
  the round trip, so the TypeScript type cannot drift from the JSON.

### Frontend (`dashboard/ui/`)

- `src/app.css`: the theme variables (background, glass card, text levels,
  accent `#3ddc97`, heatmap levels, fonts) and `.num` (monospace,
  tabular figures).
- `src/lib/Header.svelte`: presets, Custom (two date fields, shown while the
  selection is not a preset), range label ("18 Nov 2026",
  "3 – 9 Nov 2026", "November 2026", "2026"), "as of HH:MM" (with the date
  when not today), refresh (re-fetches meta, clears the series cache,
  re-fetches series and summary).
- `src/lib/Heatmap.svelte`: the SVG grid, scaled to the card width. Colour
  levels are quartiles of the days with a value (one outlier does not wash
  out the rest). Today has an accent outline, future days are blank, the
  selection is outlined. Click / drag / shift-click; the summary is fetched
  only when a drag ends. Arrow keys move the selection (shift extends).
  Hover tooltip with the date and the formatted value. Days with data and,
  for summed metrics, the year's total next to the dropdown.
- `src/lib/RangePanel.svelte`: the temporary panel: per collector that is
  on, its metrics without keys and the change vs the previous period; top
  keys; records; which collectors are off.
- `src/App.svelte`: state and data flow. Series are cached per
  (year, metric) until refresh; responses to superseded requests are
  dropped. Presets and date fields move the heatmap to the year the range
  ends in; heatmap selections never change the year.
- Empty states: no database ("No data yet", with the path), all collectors
  off, no data in the range, command errors shown inline.

### Collector

- Tray: "Open Dashboard…" (⌘D) first in the menu.
- `platform::open_dashboard(db)`: macOS `open`s the nested app with
  `--args --db <db>` (arguments only apply to a fresh launch); a bare
  binary (`cargo run`) starts `wrapped-dashboard` next to it. Windows /
  other: spawns `wrapped-dashboard.exe` next to the collector exe.
- `wrapped-collector dashboard`: the same from a shell (the only way on
  Windows until the tray exists there).

### Bundle and install

- `scripts/macos/bundle.sh` builds the frontend (`npm ci` only when the
  lockfile changed), then the collector and the dashboard in **separate**
  cargo runs (one run would unify features across both, and the collector
  must not pick up features the dashboard's dependencies enable), writes
  both Info.plists with the same versions, and signs the nested app before
  the outer bundle.
- `scripts/macos/install.sh` checks that the built and installed dashboard
  versions (plist and `--version`) match the collector's and HEAD, and
  quits a dashboard running from the installed app before the swap (it
  only reads; it is not relaunched).

## Develop

```sh
cd dashboard/ui && npm ci                     # once
npm run tauri -- dev -- -- --db ~/Documents/wrapped-notes/m3-scratch.db --debug
                                              # Vite + debug build, hot reload
npm run check                                 # svelte-check
UPDATE_TS_TYPES=1 cargo test -p wrapped-dashboard --test ts_types   # after changing src/types.rs
```

`cargo build/clippy/test -p wrapped-dashboard` work without `ui/dist`
(dev builds load the Vite server). Only `--features custom-protocol`
(used by `bundle.sh`) embeds `ui/dist`, so build the frontend first.

## Tests

- `dashboard/tests/commands.rs` (fixture database written through the real
  store and rollups in a temp folder: app sessions across a new year and
  over midnight, an open session, idle and sleep, commits, photos turned
  off): `meta` (collectors, metrics only of collectors that are on, earliest
  date, last flush; defaults without a database, which is not created),
  `series` (365 / 366 values, values on the right days, sum = summary
  total, hidden / keyed / malformed metrics refused), `summary` (totals,
  hidden collector, streaks, the TypeScript type round trip, errors), and
  that the commands neither change the database or its WAL nor block a
  `wal_checkpoint(TRUNCATE)` from a writer.
- `crates/collector/tests/dashboard_json.rs`: runs the real
  `wrapped-collector stats range --json` and the dashboard's `summary()` on
  the same fixture for four ranges (across a new year, one day, a full year,
  a year without data) and compares the JSON. `wrapped-dashboard` is a
  dev-dependency of the collector only (not linked into the binary).
- `dashboard/tests/ts_types.rs`: generated TypeScript types are up to date.
- `dashboard/tests/perf.rs` (ignored): command timings over the M4
  synthetic year.

## Performance (Apple M2 Pro, 2026-11-18)

`cargo test -p wrapped-dashboard --release --test perf -- --ignored --nocapture`,
synthetic year (all collectors, 15.5 MB), each call including the
read-only open and close; 7 runs, two rounds at normal load:

| Command | Median | Max |
|---|---|---|
| `meta` | 0.33–0.35 ms | 0.54 ms |
| `summary`, full year (365 days) | 21.3–23.0 ms | 25.4 ms |
| `summary`, calendar year 2026 | 19.6–22.0 ms | 22.5 ms |
| `summary`, one month | 3.0–3.3 ms | 3.6 ms |
| `series` 2026, screen time | 1.7–2.1 ms | 2.2 ms |
| `series` 2026, bytes written | 1.6–2.0 ms | 2.4 ms |
| `summary`, full year, one connection kept open (comparison) | 20.3–20.9 ms | 22.4 ms |

Opening per call costs about 1 ms on a full-year summary. (Under heavy
unrelated load, load average ~150, the same summary took 31–85 ms median.)

App, against `~/Documents/wrapped-notes/m3-scratch.db`:

- Cold start to window shown (`open` on the nested app, from before `open`
  to the `ready` log): **495–510 ms** (3 runs); the first launch of a freshly
  built bundle took 3.2 s (signature check, Launch Services registration,
  WebKit's first start). The process itself reaches `ready` ~400 ms after
  `main` (WebView creation and page load ~380 ms, the three commands
  < 15 ms together).
- Memory (`footprint`, window open): dashboard process **31 MB**; WebKit's
  helper processes WebContent 91 MB, GPU 45 MB, Networking 6 MB (all exit
  with the window).
- Nested app on disk: 12.5 MB (12.7 MB executable, release profile
  unchanged: no LTO, symbols not stripped).
- Collector release binary: 3,874,560 B before → 3,894,384 B after
  (+19.8 KB: the tray item and spawning the dashboard process).

## What was built (5b)

- **Fair comparisons.** `Stats::summary_vs(range, vs, top_n)`: an optional
  explicit comparison range (`summary` is `summary_vs(.., None, ..)`, so
  `stats range --json` and the dashboard stay identical). The frontend
  (`summaryQuery` in `dates.ts`) asks for a preset that contains today
  (This week / month / year) only up to today and compares it with the same
  elapsed days of the period before: Mon..today vs last Mon..same weekday; the
  1st..today vs the 1st..the same day of last month (clamped to its length);
  Jan 1..today vs Jan 1..the same date last year (Feb 29 → Feb 28). The
  heatmap selection is still the whole period (future days blank). Today,
  custom ranges and finished periods compare with the previous range of equal
  length, as before. The comparison range can be shorter than the current one
  after clamping (31 Mar vs 1..28 Feb).
- **Cards** replace `RangePanel.svelte`: a 12-column grid, S = 3, M = 6,
  L = 12 columns (S pairs up below 1100 px). `Card`, `BigNumber`, `BarList`,
  `Ring`, `HourBars` are hand-drawn SVG with colours from `app.css` only.
  Order: Screen time, Away, Time of day, Top apps, Records, Git, Photos,
  Disk & Swap, Memory pressure. Cards of collectors that are off are not
  drawn; a card without data says "Nothing in this range".
- **Friendly camera and lens names** (`photos::friendly_camera`,
  `friendly_lens`): applied by the stats summary only (stored keys stay as
  the EXIF said); keys that share a name add up. Only the owner's two
  cameras are mapped (Sony ILCE-7RM5; the DJI Air 3S's two cameras, codes
  FC9113 and FC9184). Lenses: the Air 3S's two ("24.0 mm f/1.8" → "Air 3S
  wide (24mm f/1.8)", "70.0 mm f/2.8" → "Air 3S tele"), Sigma's
  "<spec> | <line> <n>" → "Sigma <spec> <line>", and "(unknown)" / "----"
  → "No lens info". Shots with no lens EXIF cannot be tied to a camera in the
  per-lens rollup, so DJI shots without lens info are in "No lens info".
- **App names**: the `app_names(ids)` command resolves bundle ids to the
  installed app's display name (`NSWorkspace`, then the bundle's
  `CFBundleDisplayName` / `CFBundleName`), cached per process; ids with no
  installed app and non-bundle keys ("idle") stay as they are; Windows
  returns ids. The summary JSON is unchanged; tooltips keep the bundle id.
- **Today**: part of a day is compared with all of yesterday, so the change
  is shown neutral (no colour, no arrow) as "… so far", tooltip "vs all of
  yesterday". Other ranges are unchanged.
- **Away (awake)**: time away minus the "sleep" reason
  (`away_secs_by_reason`). "Times away" still counts every period (the
  per-reason period count is not stored). See DEVIATIONS.md.

## Opening the dashboard

- the menu-bar icon: "Open Dashboard…" (⌘D);
- Spotlight / Finder / `open ~/Applications/Wrapped.app` while Wrapped is
  running: the app delegate's `applicationShouldHandleReopen:` opens the
  dashboard (brings it to the front if open). A fresh launch (at login, or
  the relaunch at the end of `install.sh`) does not open it: a launch-at-login
  signal could not be verified here, so only reopen opens it;
- `wrapped-collector dashboard` from a shell.

## Refresh

The refresh button first runs the sibling `wrapped-collector flush` (SIGUSR1
to the running collector; the dashboard never writes), then polls `meta` every
100 ms until the last-recorded time changes or 1.5 s pass, then re-fetches
meta, series and summary. With no collector running it just re-fetches. The
button spins meanwhile; "as of" is highlighted briefly when its text changes.

## Known limitations / left for 5b

- No live update while open: data changes appear on refresh.
- Windows: no single-instance handling (a second "dashboard" opens a
  second window) and no default database path (pass `--db`).
- `last_flush_ms` is the latest moment recorded, which equals the last
  flush whenever something was being tracked; while paused or idle with no
  open session it is the end of the last recorded activity.
- Release size: LTO / stripping could shrink the dashboard executable;
  not done to keep the collector's build unchanged.

## 5c: polish

- **Date range picker.** The two native date inputs of Custom are replaced by
  one button showing the range ("1 Jan 2026 – 31 Dec 2026"; one day shows just
  that day). It opens `DateRangePicker.svelte`: two months side by side,
  Monday-first, prev / next arrows, a "Today" link, today ringed, the range as
  an accent band with solid end caps, future days dimmed and not selectable.
  Click a start day, then an end day (swapped when before the start; hover
  previews). Escape or a click outside cancels, leaving the selection as it
  was; Return or the second click applies, through the same `oncustom` path as
  the old inputs. Keyboard: arrows move the focused day, shift+arrows extend,
  Space picks like a click, Return applies. The popover is absolutely
  positioned (no header shift) and closes when a preset is chosen.
- **Heatmap selection.** No per-cell borders: selected cells keep their
  colour, the rest drop to 35% opacity; a single day gets a thin accent ring.
- **Away (awake) card.** "Times away" is removed (it counted sleep periods
  too; only the daily total is stored). Stored data and CLI output are
  unchanged. The card now shows two rows with a colour key matching the ring.

No deviations from DESIGN.md, so DEVIATIONS.md is unchanged.

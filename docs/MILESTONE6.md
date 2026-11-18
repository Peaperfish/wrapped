# Milestone 6: onboarding, settings and AI usage

DESIGN.md §12 (first-launch checklist, Settings) plus a new scanned collector,
**AI usage**, which imports token counts from the local logs of AI coding
tools. The research behind the collector is in [AI_SOURCES.md](AI_SOURCES.md).

Status: **plan** (phase 1). "What was built" follows in phase 2.

## Baseline (main @ 23cfa9c, 2026-11-18)

Measured before any M6 code, for the before / after comparison:

| What | Before |
|---|---|
| Collector release binary (installed) | 3,895,360 B |
| Collector memory, idle (`footprint`, installed app, running) | 20 MB (peak 21 MB), RSS 73 MB |
| Dashboard bundle (`ui/dist/assets`) | JS 75,695 B + CSS 11,339 B (fresh `npm run build`) |
| Real database | 1.14 MB (+ 21 KB WAL) |

Collector memory is already at the DESIGN.md §4 budget (< 20 MB), so the AI
scan must not raise idle memory: it runs on the scan thread, streams files line
by line, and frees everything when it is done.

## Plan (decisions)

### 1. Command channel: dashboard → collector

The collector stays the only writer; the dashboard keeps opening the database
read-only.

- **Transport** (the same idea as `flush`, which is `wrapped-collector flush` →
  SIGUSR1): the dashboard runs the sibling `wrapped-collector ctl` with one JSON
  request on stdin and reads one JSON reply from stdout. `ctl` writes the
  request to `<data dir>/ctl/<uuid>.json` (written to a temp name, then renamed
  into place), sends **SIGUSR2** to the pid in `wrapped.lock`, and waits up to
  5 s for `<uuid>.reply.json`. The collector's signal thread hands SIGUSR2 to
  the main queue, which handles every pending request in order (the engine is
  only touched on the main thread, as now), writes the reply and deletes the
  request. Requests older than 30 s are deleted unanswered (the client has
  given up).
- **No collector running:** `ctl` exits with code 3 and
  `{"error":"not_running"}`; the dashboard shows "Wrapped isn't running. Open
  Wrapped to change settings." with an "Open Wrapped" button (it runs `open`
  on the outer app; it never quits or restarts anything). A timeout is a
  separate error ("Wrapped didn't answer").
- **Requests** (typed in `wrapped_core::ctl`, shared with the CLI):
  `set_settings {pairs}` (applied in one transaction with the same validation
  and side effects as `settings set`: defaults on enable, fingerprint reset),
  `pause {kind}`, `scan {what}`, `delete_collector_data {name}`,
  `delete_all {confirm: "delete"}`, `add_ai_source {path, tool}`,
  `remove_ai_source {id}` (only on the user's explicit request),
  `finish_setup {pairs}`, `status` (setup mode, pause, scans running, AI source
  states).
- **Read-only things stay in the dashboard** (no channel): exports (the
  dashboard backend calls `Store::export_json` / CSV on its read-only
  connection, writing to a file the user picked) and the git email list
  (`wrapped-collector git emails --json <folders>`, read-only, extended to take
  folders so setup can list emails before anything is saved).
- **CLI `settings set`** goes through the channel too when a collector is
  running (today it writes beside it); with no collector it writes directly
  while holding the single-writer lock. One writer either way.
- **Windows:** `ctl` returns `not_running` until the Windows collector exists
  (milestone 9 adds a named event in place of SIGUSR2).

### 2. Setup mode (first-launch checklist)

- New setting `onboarding.done` (bool, default false).
- **Existing installs:** at startup, if `onboarding.done` is unset and the
  database has any recorded data (sessions, samples, items or daily metrics),
  the collector stores `onboarding.done = true` and nothing else. Settings are
  never reset or rewritten. That covers this Mac.
- **Fresh install** (`onboarding.done` unset, no data): the collector starts in
  **setup mode**: the engine runs but records nothing and starts no scans
  (handled like a pause that the tray cannot lift), the tray shows "Finish
  setup…", and the dashboard opens. A fresh launch from Spotlight / Finder
  opens the dashboard too, but only in setup mode (normal fresh launches,
  e.g. at login, still don't).
- **Dashboard setup screen** (full window, same dark glass look), one row per
  collector from `info()`: what it records, the permission it needs, its cost.
  Ticked by default: App usage, Active / idle, AI usage. Unticked: Git,
  Photos, Disk & Memory.
  - Git ticked → folder list (default `~/code`, "Add folder…") and the author
    emails found in those folders as checkboxes (yours from `git config`
    pre-ticked), the same list as `git emails`.
  - Photos ticked → folder list (default `~/Pictures`).
  - AI usage → the tools detected on this machine with their status ("Claude
    Code: 369 sessions found", "Cursor CLI: no token counts on disk").
  - "Start recording" sends `finish_setup` with every setting in one request.
    Recording starts there; the first scans start right after.
- Closing the window without confirming leaves setup mode on; the tray's
  "Finish setup…" opens it again.
- "Never tracked" line at the bottom (§12): keystrokes, screen contents,
  window titles, file contents, prompt / response text.

### 3. Settings screen

- Gear button at the top right of the header (next to refresh) and a
  "Settings…" tray item (⌘,). The tray item writes a flag file
  `<data dir>/ctl/open-settings` and opens the dashboard as today; the
  dashboard checks and removes the flag at launch and whenever its window
  gains focus, and switches to Settings. (No single-instance plugin needed:
  macOS already brings a running instance to the front.)
- Same window, a panel replacing the cards (header stays), dark glass sections:
  - **Tracking:** a toggle per collector (turning one off asks "Keep or delete
    its data?"), pause 1 h / until tomorrow / indefinitely, resume.
  - **Exclude list:** apps never recorded. Add from the apps already recorded
    (names via `app_names`) or by bundle id; remove.
  - **Folders:** Git folders and author emails, Photos folders; "Add
    folder…" opens the native folder picker.
  - **AI sources:** every source with tool, path, status ("OK, 1,234
    requests", "Not found, last seen 12 Nov", "No token counts on disk"), an
    on / off switch, "Scan now", and "Add folder…" with a tool picker.
  - **Data:** export JSON / CSV (save dialog), delete all data (typed
    confirmation: the user types `delete`).
- Every change is one channel request and applies without a restart (the
  engine already reloads settings on change; this calls that reload at once
  instead of at the next minute tick).

### 4. AI usage collector

- `crates/core/src/collectors/ai_usage.rs` (collector `ai_usage`, Scanned,
  default **on**, no permission) and `crates/core/src/ai/` with one file per
  adapter: `claude.rs`, `antigravity.rs` (tested), `codex.rs`, `gemini.rs`,
  `copilot.rs`, `cursor.rs`, `opencode.rs` (each with
  `pub const TESTED: bool = false` and a doc comment saying so).
- **When:** at startup, daily (the existing scan scheduler), on dashboard
  Refresh (refresh sends `scan ai_usage` then flush; the scan is incremental,
  so this is cheap), and `wrapped-collector scan ai` (alias of `ai_usage`).
- **Sources** (setting `ai.sources`, a JSON array of
  `{id, tool, path, builtin, enabled, bookmark?}`):
  - Built-in sources are the default path of each tool (env var overrides
    applied at scan time: `CLAUDE_CONFIG_DIR`, `CODEX_HOME` incl. its comma
    list, `COPILOT_HOME`, `GEMINI_CLI_HOME`, `XDG_DATA_HOME`,
    `OPENCODE_DATA_DIR`). A built-in source is added to the setting the first
    time its folder is found, and never removed automatically.
  - Custom sources: "Add folder…" + the tool whose format it holds.
  - Per-source state in `scan_state.state` (JSON keyed by source id):
    `last_seen_ms`, `missing_since_ms`, counts for the status line. A missing
    folder shows "Not found, last seen <date>" and is checked on every scan;
    when it comes back, scanning resumes from the stored file positions.
  - **macOS bookmarks for custom folders:** the collector creates an
    `NSURL` bookmark (`bookmarkDataWithOptions`, not security-scoped: the app
    isn't sandboxed) when the source is added. If the path is missing at scan
    time, the bookmark is resolved; if it points to an existing folder, the
    path is updated quietly (and the bookmark refreshed when stale). Windows:
    path only.
- **Storage** (schema v4):
  - `ai_usage(tool, hour_ms, off, session, model, project, input, output,
    cache_read, cache_write, requests, PRIMARY KEY (tool, hour_ms, session,
    model, project)) WITHOUT ROWID`: requests summed per UTC hour (with the
    local offset of the first request), tool, session, model and project.
    Upserted additively in the same transaction as the dedupe rows.
  - `ai_seen(tool, time_ms, id_hash, PRIMARY KEY (tool, time_ms, id_hash))
    WITHOUT ROWID`: one 64-bit hash per counted request (message id, step,
    cumulative total…). Keyed by time first, so new requests append to the
    same few pages and an old repeat is a single seek. A request is added to
    `ai_usage` only if its `ai_seen` row was new.
  - `scan_files` gains `pos` (byte offset after the last complete line) and
    `head` (hash of the first line) columns, NULL for photos. Keys stay salted
    path hashes; no paths are stored except the source folders in settings.
  - Daily metrics (`ai_usage` collector): `tokens` (input + output, the
    heatmap metric), `tokens_input`, `tokens_output`, `tokens_cache_read`,
    `tokens_cache_write`, `requests`, `sessions`, keyed rows
    `tokens_by_tool`, `tokens_by_model`, `tokens_by_project`,
    `requests_by_tool` (for tools without token counts) and `active_hours`.
    Rebuilt from `ai_usage` by `recompute_day` like the other collectors.
  - Only time, tool, model, the four token counts, session id and the
    project folder name. No text, no full paths, no cost.
- **Why not one `items` row per request:** Claude Code alone wrote ~550
  requests a day here (11,471 in 21 days). One items row each (~250 B with
  the ids) is ~50 MB a year and random B-tree inserts; the hourly table plus
  time-ordered 25 B dedupe rows stays within the 1-2 MB/day SSD budget. See
  DEVIATIONS.md (the design says scanned collectors store items).
- **Dedupe, positions, rotation:** see AI_SOURCES.md "Shared rules". Resumed /
  forked sessions are covered by the global `ai_seen` (392 Claude message ids
  appear in more than one file here). Cumulative counters (Codex, Copilot
  checkpoints) become deltas, keyed by `(session, cumulative total)`; the last
  totals per session live in the file's state so a resumed read continues
  correctly. A file that shrank or whose head changed is read again from the
  start. Imported rows are never deleted when a log disappears.
- **Tools without token counts** (Cursor CLI): rows with zero tokens and
  `requests` = sessions; the card says "Cursor CLI: sessions only (no token
  counts on disk)".
- **Privacy in code:** adapters deserialize into structs that only have the
  allowed fields (serde ignores the rest), so text never exists outside the
  JSON parser's scratch buffer. The project name is the last component of the
  path, cut inside the adapter. Tests use made-up fixtures only.
- **CLI:** `wrapped-collector scan ai`, `wrapped-collector ai sources`
  (detected / configured sources and their status, no file names).

### 5. Dashboard card

- "AI usage" card (L, after Photos): big numbers for tokens (input + output)
  with the change vs the previous period, cache read and cache write next to
  it; bars by tool and by model; top projects; active days. A note per tool
  without token counts. Hidden when the collector is off; "Nothing in this
  range" otherwise. "AI tokens" in the heatmap metric dropdown.

### 6. Dependencies

- **New:** `tauri-plugin-dialog` (Rust) and `@tauri-apps/plugin-dialog`
  (npm), the official Tauri dialog plugin, for the folder picker and the
  export save dialog. Dashboard only; the collector gets nothing new.
- Collector: bookmarks use the existing `objc2-foundation` (`NSURL`
  features), JSON parsing uses the existing `serde_json`. SQLite sources
  (Cursor, opencode's db) use the existing `rusqlite`.

### 7. Tests

- Core: each adapter on made-up fixture logs (including repeated lines,
  resumed-session copies, cumulative counters, a partial last line, a
  truncated / replaced file, a deleted log, a missing then returning folder);
  rescans count nothing twice; the schema v4 migration on a v3 database
  keeps every row.
- Settings / ctl: request round trips, setup mode records nothing until
  `finish_setup`, an existing database gets `onboarding.done` and no other
  change, the CLI without a collector.
- Dashboard: `meta` exposes setup mode, AI metrics and the AI card data; ts
  types; the read-only guarantees still hold.
- Fresh-install checks only on scratch databases (`--db` / `WRAPPED_DB`).

### 8. Measurements (phase 2)

Before vs after: AI scan time on the real logs (first scan, rescan), database
growth from the import, collector idle memory, collector binary size,
dashboard bundle size (JS + CSS). Real-log measurements read the logs only
through the collector on a **copy** of the real database, so the real one is
untouched until the installed app does its own first scan.

## Open questions

1. **Heatmap / headline metric.** Cache reads dwarf everything else (in
   Claude Code they are typically 10-100× input + output). Plan: the headline
   "tokens" and the heatmap use input + output, with cache read / write shown
   separately. Or do you want a grand total including cache?
2. **Antigravity model.** The transcript has no model; model ids are only
   inside protobuf blobs of `conversations/<uuid>.db`. Plan: record "Gemini
   (Antigravity)" as unknown model for now. Alternative: best-effort pull of
   the model id string from those blobs (works today, may break on any
   update).
3. **"Delete all data"** keeps settings and `onboarding.done` (so it doesn't
   drop you back into setup mode) and resets the AI file positions, so the
   next scan imports again whatever logs still exist. OK? (Older usage whose
   logs are gone would then be gone for good.)
4. **Dedupe rows forever?** `ai_seen` grows ~5 MB a year at the current rate.
   Plan: keep forever (a log restored from a backup months later must not
   count twice). Alternative: prune rows older than 400 days.
5. **Built-in sources not found.** Plan: Settings → AI sources lists all
   seven tools, the ones never found greyed as "Not installed" (checked on
   every scan, added as a source once found). Setup lists only the detected
   ones. Or should undetected tools be hidden in Settings too?

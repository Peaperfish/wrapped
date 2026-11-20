# Wrapped — Design Doc

| Status | *milestones 0–8 built (collector, stats engine, dashboard, setup, Now tab, recaps); 9 implemented pending Windows verification; 10 open* |
| :---- | :---- |
| **Platforms** | *macOS and Windows* |

## 1\. Overview

A local-only desktop app that quietly tracks how I use my computer and turns it into:

- an always-available **dashboard** (any date range),  
- a **Now** tab with live hardware stats,  
- **weekly, monthly and yearly recaps** (Spotify Wrapped-style story slides).

## 2\. Goals

- Fun to look at, useful every day.  
- Every data source is **opt-in** and can be turned off/deleted at any time.  
- New data sources can be added as **collector plugins** without touching the rest of the app.  
- Runs on **macOS and Windows**.

## 3\. Non-goals

- No cloud, accounts, or uploading. Data never leaves the machine.  
- No keystroke logging (content or counts), clipboard, screenshots, URLs or file contents.  
- Not a productivity/“focus” nagging tool.

## 4\. Hard requirements (performance budgets)

| Area | Budget | How to verify |
| :---- | :---- | :---- |
| Battery (Mac, unplugged) | Energy impact ≈ 0 | Activity Monitor → Energy |
| Gaming (Windows) | No measurable FPS change | CS2 benchmark, collector on vs off |
| RAM (collector) | \< 20 MB | Activity Monitor / Task Manager |
| CPU (collector) | \~0% when idle or in one app | Activity Monitor / Task Manager |
| SSD writes | \~1–2 MB/day | Activity Monitor → Disk (bytes written) |

Rules that make this possible:

- Collector is a **plain Rust process \+ native tray icon** (no web view).  
- **Event-driven** app tracking (OS app-switch notifications), not polling.  
- Idle and counter checks at most **once per minute / every 5 min**.  
- Events **buffered in memory**, flushed every \~10 min \+ on sleep/shutdown.  
- Dashboard/recap is a **separate Tauri window**, only running while open.

## 5\. Architecture

```
Collector (always on, tray)  ──▶  SQLite (local)  ◀──  Dashboard app (Tauri, on demand)
   ├─ live collector plugins                            ├─ Overview (heatmap + cards)
   └─ shared batched writer                             ├─ Now tab (live hardware)
                                                        └─ Recaps (week / month / year)
Scanned collectors (git, photos, drive health) run on demand / daily.
```

## 6\. Collector plugin system

Every data source — including built-in ones — is a collector implementing one trait:

```rust
trait Collector {
    fn info(&self) -> CollectorInfo;            // name, description, permissions, cost, default on/off
    fn kind(&self) -> Kind;                     // Live or Scanned
    fn start(&mut self, out: Sink);             // Live: emit events
    fn scan(&mut self, since: Time, out: Sink); // Scanned: read new data since last scan
    fn stop(&mut self);
    fn metrics(&self) -> Vec<MetricDef>;        // what it adds to dashboard/recaps
}
```

Framework provides to every collector automatically:

- Onboarding \+ Settings toggle (built from `info()`).  
- Storage via the **shared batched writer** (collectors never write to disk directly).  
- **Daily metrics** table → heatmap, date ranges and stats engine work for any metric.  
- **Generic card types**: Top-N list, Big number \+ change vs previous period, Time-of-day chart.  
- Recap slides, “turn off”, “delete this data”.

Adding a collector \= 1 new file \+ 1 registration line. Later (optional): external collectors as separate programs talking JSON over stdin/stdout. No .dylib/.dll loading.

## 7\. Collectors

| Collector | Type | Records | Permission | Default | Phase |
| :---- | :---- | :---- | :---- | :---- | :---- |
| App usage | Live | Front app \+ start/end times | None | On | v1 |
| Active / idle | Live | Away periods | None | On | v1 |
| Git | Scanned | Commit times \+ repo names (no code) | Pick folders \+ author emails | Off | v1 |
| Photos | Scanned | EXIF only (date, camera, lens, settings) | Pick folders (Mac asks once) | Off | v1 |
| AI usage | Scanned | Token counts per hour, tool, model and project folder name from AI coding tools' local logs (never prompt or response text) | None (reads the tools' own folders) | On | v1 |
| Disk & Memory | Live (5 min) | Disk GB read/written, swap in/out, memory pressure, compressed memory (Mac) | None | Off | v1 |
| Hardware history | Live (5 min avg) | CPU/GPU load \+ temps | None | Off | v1.x |
| Network usage | Live | MB up/down per day | None | Off | v1.x |
| Window titles | Live | Window titles (project/site stats) | Mac: Screen Recording  | Off | Later |
| Disk writes per app | Live | Per-app read/write (Mac, user apps) | None | Off | Later |
| Drive Health | Scanned (daily) | Lifetime TB written/read, wear %, temp, power-on hours, projected lifespan | Windows: admin (separate elevated scan) | Off | Later |
| Planes spotted | Live | From future flight-tracker project | — | Off | Someday |

## 8\. Data model (SQLite)

- `sessions(collector, key, start, end)` — e.g. app usage sessions.  
- `samples(collector, metric, time, value)` — periodic counters (disk, swap, temps).  
- `daily_metrics(date, collector, metric, key, value)` — rollups used by dashboard/recaps.  
- `scan_state(collector, last_scanned)` — for incremental git/photo scans.  
- `settings` — enabled collectors, exclude list, pause state. Counters that reset on reboot (disk, swap) → store deltas, detect resets and re-baseline.  
- Time zones: store every timestamp in UTC plus the local UTC offset at that moment; a session’s day is decided by its local time (handles travel, e.g. Shenzhen → Toronto).  
- Database has a schema version number; updates run migrations so old data is never lost.

## 9\. Dashboard

- **Top:** GitHub-style year heatmap. Click a day, drag/shift-click for a range. Presets: Today · This week · This month · This year · Custom. Arrows for previous years. Heatmap colour metric is selectable (screen time, GB written, commits, …).  
- **Cards** (update for the selected range): Screen time · Top apps · Categories · Time of day · Git · Photos · Disk & Swap · Memory pressure · Records (longest session, latest night, streaks).  
- Cards for disabled collectors are hidden.  
- Style: dark glass cards, rings and bars (hardware-monitor look).

## 10\. Now tab

Live CPU/GPU/RAM, temps, fans, disk read/write speed, network speed, swap. Updates only while the window is open. Saves nothing unless “Hardware history” is enabled.

## 11\. Recaps

- Weekly (Mondays), monthly (1st), yearly (December). Optional “Your week is ready” notification.  
- Story-style full-screen slides; same stats engine with a different date range.  
- Export slides as PNG (video later).

### AI usage (Milestone 6)

- Reads the local logs of Claude Code, Codex, Gemini CLI, Google Antigravity,
  Copilot CLI, Cursor CLI and opencode (research and per-tool formats:
  [AI_SOURCES.md](AI_SOURCES.md)). Only Claude Code and Antigravity are tested
  against real logs; the rest are written from documentation.
- Stored per UTC hour, tool, session, model and project folder name: input,
  output, cache read and cache write tokens and a request count (table
  `ai_usage`). A small `ai_seen` table of 64-bit hashes stops a request that
  appears in several files (resumed sessions, repeated lines) from counting
  twice. Rows are never deleted when a log disappears. No text, no full paths.
- Headline and heatmap = input + output tokens; cache read / write are shown
  next to it, not added in (cache reads are 10-100x the rest).
- Scans run at startup, daily, on dashboard Refresh and with
  `wrapped-collector scan ai`; they are incremental (stored byte offsets).
  Sources are listed in Settings → AI sources (add a folder, switch one off,
  scan now); custom folders follow a move through a macOS bookmark.
- Tools that log no token counts (Cursor CLI) show sessions only.

## 12\. Privacy & control

- First-launch checklist: each collector shows what it records, permission needed, battery/disk cost.  
- Settings → Tracking: toggle anytime; turning off asks “keep or delete existing data?”.  
- Exclude list (apps never recorded).  
- Pause from tray: 1 hour · until tomorrow · indefinitely.  
- “Delete everything” button.  
- Export all data to CSV / JSON.  
- App states clearly what is never tracked.

## 13\. Platform notes

**macOS**

- Front app via system app-switch notifications; idle via “seconds since last input” — no permissions.  
- Folder access prompts (Documents/Pictures/external drives) — one-time.  
- Launch at login shows a one-time “background item added” notice.  
- Locally built apps aren’t blocked by Gatekeeper; matters only if distributed.

**Windows**

- Front window via out-of-context `SetWinEventHook` (gaming impact requires measurement).
- Idle via `GetLastInputInfo`. Collector runs at below-normal priority.  
- UWP apps appear as `ApplicationFrameHost.exe` → resolve real app.  
- No keyboard hooks (avoids antivirus flags).  
- GPU temps and Drive Health need extra work / admin.

**Both**

- Idle detection: default idle after \~3 min without keyboard/mouse (adjustable in settings).  
- Users can tick apps that should count as active without input (e.g. PDF reader, notes, video player — for studying/reading). Those use a longer timeout (default 30 min, adjustable), so an app left open overnight still goes idle.  
- Mac: an app holding a “keep display awake” request (video playback) also counts as active — no permission needed. Windows: reading this needs admin, so the ticked-apps list is the main method there.  
- Sleep, lid close or display sleep always ends the current session (no 8-hour overnight sessions).

## 14\. Tech stack

- Collector: Rust (`sysinfo` for cross-platform system stats, platform APIs for app switching), SQLite (`rusqlite`).  
- Dashboard / recaps: Tauri (Rust \+ HTML/CSS/JS).  
- CI: GitHub Actions building on macOS \+ Windows runners.

## 15\. Milestones

| \# | Step | Rough time |
| :---- | :---- | :---- |
| 0 | Spike: print front app \+ idle state | 1 evening |
| 1 | Collector \+ plugin system \+ SQLite \+ daily metrics \+ tracking settings \+ tray \+ launch at login | 1.5–2 weekends |
| 2 | Collectors: Git \+ Photos (incremental) | 1 weekend |
| 3 | Collector: Disk & Memory | 1 weekend |
| 4 | Stats engine (any date range) | 1 weekend |
| 5 | Dashboard: heatmap \+ cards | 2 weekends |
| 6 | Onboarding \+ Settings | 1 weekend |
| 7 | Now tab | 1 weekend |
| 8 | Recap slides \+ notifications | 1–2 weekends |
| 9 | Windows port | 1 weekend |
| 10 | Collector: Drive Health | 1–2 weekends |

**Total: \~12–14 weekends**. Priority: get the collector (0–1) running early so data accumulates.

## 16\. Open questions

- Final app name.  
- Category mapping for apps: built-in list \+ user edits?  
- Mac Drive Health access method (IOKit vs `smartctl`) — needs a spike.  
- Video export for recaps — worth it?  
- Ever distribute to others (signing, notarization, licence)?


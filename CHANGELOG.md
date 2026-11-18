# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- A collector turned off while tracking was paused, then turned on again after resuming, kept its old pause flag and recorded nothing (app usage, active / idle, disk & memory).
- Dark wakes: minute ticks between `WillSleep` and `DidWake` (macOS maintenance wakes) are logged as "tick during dark wake", not as a tick-gap warning, and skip the periodic 10-minute flush; `measure.sh` reports them as `dark_wake_ticks`.
- Collector no longer records itself as an app: `FrontApp` events for its own bundle id (e.g. opening the tray menu) are ignored by the engine.

### Added

- Milestone 4: stats engine (`wrapped_core::stats`): totals by each metric's aggregation, per-day series with zeros, top-N keys, change vs the previous period, time-of-day profiles and records (longest app session, latest night, screen-time and commit streaks) for any range of local dates, hiding collectors that are off; `MetricDef.agg`; `Store::open_read_only`; `wrapped-collector stats range <from> <to> [--json]`; a synthetic-year benchmark (`stats_perf`); and `docs/MILESTONE4.md`.
- Milestone 3: Disk & Memory collector (off by default): system-wide disk bytes read / written (internal storage), swap in / out, memory pressure and compressed memory, read every 5 minutes while awake through a platform `SystemProbe` (macOS: IOKit, Mach VM statistics, sysctl; no permission, no child process). Reboot-safe deltas (boot time), one delta per sleep gap at the wake tick, daily metrics split across days and hours, schema v3 (`samples.data`, one row per reading), `wrapped-collector probe disk-memory`, and `docs/MILESTONE3.md`.
- `write_budget` simulation runs 16 h and 24 h days with and without Disk & Memory.
- Milestone 2: scanned collectors Git and Photos (off by default), the scan framework (daily scheduling, background-priority threads, cancel/resume, early flushes with backpressure), schema v2 (`items`, `scan_files`), settings `git.folders`, `git.author_emails`, `photos.folders`, `photos.extensions`, `wrapped-collector scan [git|photos|all]`, items in export, and `docs/MILESTONE2.md`.
- `scripts/macos/install.sh`: builds, replaces the single `~/Applications/Wrapped.app` in place (graceful quit, atomic swap, relaunch, version check) and removes other `Wrapped.app` bundles from `target/`.
- Bundle versioning (`CFBundleVersion` = git hash, `-dirty` when uncommitted) and `wrapped-collector --version`.
- `CLAUDE.md` with standing repo rules.

- Milestone 1 core (`crates/core`): collector plugin trait, system event bus, presence state machine, app usage and active/idle collectors, SQLite storage with migrations, batched writer, daily metric rollups, tracking settings.
- `crates/collector` binary with a cfg-gated platform module (replaces the `crates/wrapped` placeholder).
- macOS collector app: native tray (pause / resume / open data folder / quit, no Dock icon), NSWorkspace and lock/unlock notifications, minute idle check with display-sleep assertions, flush on sleep / power-off / quit / SIGTERM / SIGINT, single-instance lock.
- Launch at login via `SMAppService` and `scripts/macos/bundle.sh` (ad-hoc signed `Wrapped.app`).
- Temporary CLI: `settings`, `stats`, `sessions`, `export --json/--csv`, `flush`, `login`; `--debug` / `WRAPPED_DEBUG=1` event, tick-gap and flush logging.
- `docs/MILESTONE1.md` (build, install, decisions, limitations, verification checklist) and `docs/DEVIATIONS.md`.
- Milestone 0 spike (macOS): front app, idle and sleep/wake tracking, in `spikes/`.
- Design doc in `docs/DESIGN.md`.
- Project files for a public repository: README, dual MIT/Apache-2.0 license, contributing guide, security policy, code of conduct, issue and PR templates, Dependabot and CI.

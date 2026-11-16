# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Dark wakes: minute ticks between `WillSleep` and `DidWake` (macOS maintenance wakes) are logged as "tick during dark wake", not as a tick-gap warning, and skip the periodic 10-minute flush; `measure.sh` reports them as `dark_wake_ticks`.
- Collector no longer records itself as an app: `FrontApp` events for its own bundle id (e.g. opening the tray menu) are ignored by the engine.

### Added

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

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
  `Time`.

## Data model (§8)

- `sessions` has extra columns: `start_off` and `end_off` (local UTC offset in
  minutes at that moment) and `open` (1 while the session was still running at
  the last flush; cleared on startup by crash recovery).
- `samples` has `time_off` (local UTC offset of the sample).
- `scan_state` has a `state` column (collector-defined JSON) next to
  `last_scanned_ms`.
- `daily_metrics.date` is the local date as text `YYYY-MM-DD`.
- `settings` holds one bookkeeping row, `internal.max_session_ms` (longest
  session ever stored), which bounds how far back a day's rollup has to look.
  Keys starting with `internal.` are not user settings.

## Active / idle collector (§7)

- Away periods include sleep. Each period is keyed by the reason it started
  with: `idle`, `sleep`, `display_off`, `locked`, `screensaver`. A period that
  starts idle and continues into sleep stays one period keyed `idle`. Pause and
  power-off record nothing.
- Extra metric `active_idle/away_secs_by_reason` (key = reason).

## Metric definitions

- `app_usage/app_switches`: number of sessions that start exactly where the
  previous session (on the same local day) ended, with a different app. A gap
  (idle, lock, an excluded app) between two apps is not a switch, and neither
  is the after-midnight continuation of a session or a split caused by a UTC
  offset change.
- `active_idle/away_periods`: number of away periods that start on that local
  day. A period crossing midnight counts once, on the day it started; a split
  caused by a UTC offset change does not start a new period.

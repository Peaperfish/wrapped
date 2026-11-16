# Milestone 4: stats engine

The stats engine behind the dashboard (DESIGN.md §9, "any date range") and
the recaps (§11): one core module, `crates/core/src/stats.rs`, pure reads,
no UI and no platform code. Accepted differences from the design doc are in
[DEVIATIONS.md](DEVIATIONS.md).

## What was built

- **Metric descriptions**: `MetricDef` now has `agg` next to `unit`. Units:
  `Seconds`, `Count`, `Bytes` (Bytes added in M3). Aggregations across days:
  `Sum`, `Max`, `Mean` (mean of the days that have a value; days without data
  are not zeros). Every metric from M1–M3 declares both (the field is not
  optional, so the compiler enforces it); `compressed_peak_bytes` is `Max`,
  `compressed_avg_bytes` is `Mean`, everything else `Sum`. The test
  `every_emitted_metric_is_declared_with_unit_and_aggregation` runs every
  collector's rollup on input that makes it emit all its metrics and fails if
  one is emitted without a definition, declared but never emitted, a
  `*_by_hour` metric is unkeyed or not `Sum`, or a `*_secs` / `*bytes*`
  metric has the wrong unit.
- **`Stats` API** over an inclusive `DateRange` of local dates
  (`DateRange::new` refuses `from > to`):
  - `totals(r)`: every visible metric and key, combined by its `agg`;
  - `series(collector, metric, key, r)`: one value per day, zeros for
    missing days (heatmap); `key = None` combines all keys per day;
  - `top(collector, metric, r, n)`: best keys first, ties by key (apps,
    repos, cameras, lenses, focal lengths, away reasons, pressure levels);
  - `compare(r)`: every metric without keys against `r.previous()` (the same
    number of days just before): current, previous, change and percent
    (`0 %` when both are zero, none when only the previous is zero);
  - `time_of_day(r)`: 24 values per `*_by_hour` metric;
  - `records(r)`: longest single app session (starting in the range; pieces
    split only by a UTC offset change count as one), latest night (latest
    local clock time with app activity; before 05:00 counts as the previous
    day's night, so 02:15 beats 23:40), longest streak of days with screen
    time and of days with commits;
  - `summary(r, n)`: all of the above in one pass over the range's rows, plus
    the per-day series of every metric without keys; `Summary::to_json()`
    for the dashboard.
  - Metrics of collectors that are off are hidden everywhere (totals, top,
    series, comparisons, time of day, records), even if their data was kept.
- **Store**: `Store::open_read_only` (no migration, no writes, refuses a newer
  schema; an older schema still reads `daily_metrics` and `sessions`),
  `each_daily_metric_between` (one primary-key range scan) and
  `each_session_of` (one primary-key range scan).
- **CLI**: `wrapped-collector stats range <from> <to> [--json]`, read-only.

## Correctness

- **Days are local dates as stored.** Every rollup already assigns each
  second, byte and item to exactly one local date (sessions by their start
  offset and split at local midnight, items by their own offset, samples by
  their window). The engine sums rows of distinct dates, so a range spanning
  a DST change or a flight counts each second once. Verified end-to-end
  (`dst_and_offset_change_count_each_second_once`): 7 h of wall-clock use
  across a fall-back and a jump to UTC+1 total exactly 25,200 s, per day and
  per hour; the longest session is the whole 7 h (offset splits joined).
- **Photos without an offset** count on the camera's date and hour.
- **Empty ranges**: a reversed range is an error; a range with no data gives
  empty totals and comparisons, all-zero series and profiles, no records.
  Before any data: same.
- **Single day and a full (leap) year**: the year total equals the sum of
  366 single-day totals and the number of stored commits; top apps sum to
  screen time.

## Performance (release build, Apple M2 Pro, 2026-11-16)

`cargo test -p wrapped-core --release --test stats_perf -- --ignored --nocapture`

Synthetic year through the real store and rollups: 79,818 sessions, 5,284
items, 70,080 samples, 29,443 `daily_metrics` rows; database 14.8 MB.
Reopened read-only (OS cache warm); each operation 7 runs.

| Operation | Median | Max |
|---|---|---|
| year totals (all metrics, all keys; 111 results) | 6.25 ms | 6.34 ms |
| year heatmap series, screen time (365 days) | 1.28 ms | 1.35 ms |
| year heatmap series, bytes written | 1.24 ms | 1.29 ms |
| year top-10 apps | 2.94 ms | 2.98 ms |
| year top-5 repos | 1.32 ms | 1.35 ms |
| year compare vs previous year | 6.22 ms | 6.30 ms |
| year time of day | 6.17 ms | 6.26 ms |
| year records (longest session, latest night, streaks) | 13.24 ms | 13.66 ms |
| year summary (everything) | 19.79 ms | 20.44 ms |
| month summary (everything) | 2.28 ms | 2.36 ms |

All under the 50 ms target, so **no index was added**: `daily_metrics`
queries are a range scan on the primary key `(date, …)` (query plan:
`SEARCH daily_metrics USING PRIMARY KEY (date>? AND date<?)`), and records
scan `sessions` on `(collector, start_ms)`.

## `stats range` on the M2 scratch database

`~/Documents/wrapped-notes/m2-scratch.db` (real git and photos data), opened
read-only; `shasum` of the database and WAL identical before and after (only
the `-shm` index's mtime changes, as for any WAL reader).

App usage and active / idle are on by default but this database has no
sessions (it was only used for scans), so they show no data; disk_memory is off.

```
$ wrapped-collector stats range 2025-11-17 2026-11-16
Stats for 2025-11-17 .. 2026-11-16 (365 days), compared with 2024-11-17 .. 2025-11-16
Hidden (collector off): disk_memory

app_usage
  (no data)

active_idle
  (no data)

git
  commits                               644   previous                0   new
  top commits_by_repo
    1. prism                                           580
    2. wrapped                                          44
    3. lightcraft                                       20
  per day commits              15 of 365 days > 0, highest 2026-10-30 (71)

photos
  shots                                1693   previous              656   +158.1%
  top shots_by_camera
    1. SONY ILCE-7RM5                                 1571
    2. DJI FC9113                                      100
    3. DJI FC9184                                       22
  top shots_by_lens
    1. 24-70mm F2.8 DG DN II | Art 024                 588
    2. FE 200-600mm F5.6-6.3 G OSS                     492
    3. E 70-300mm F4.5-6.3 A047                        273
    4. FE 20mm F1.8 G                                  139
    5. (unknown)                                        90
  top shots_by_focal_length
    1. 70                                              272
    2. 600                                             175
    3. 20                                              139
    4. 300                                             115
    5. 9                                               100
  per day shots                30 of 365 days > 0, highest 2026-06-29 (253)

Time of day (local hours 00..23)
  git/commits_by_hour                |        ▃█▆▆▂▂███▄▂▃▂▁▂ |  peak 16:00
  photos/shots_by_hour               |▅▂ ▂▂▄▁  ▂▃█▁▃▂▃▂▅▆█▄▆▂▃|  peak 19:00

Records
  longest commit streak  6 days (2026-11-09 .. 2026-11-14)

$ wrapped-collector stats range 2026-11-09 2026-11-15
Stats for 2026-11-09 .. 2026-11-15 (7 days), compared with 2026-11-02 .. 2026-11-08
Hidden (collector off): disk_memory

app_usage
  (no data)

active_idle
  (no data)

git
  commits                               255   previous              206   +23.8%
  top commits_by_repo
    1. prism                                           255
  per day commits              6 of 7 days > 0, highest 2026-11-13 (58)

photos
  (no data)

Time of day (local hours 00..23)
  git/commits_by_hour                |        ▃▇▇▅▃▂▇▆█▃▃▆▄ ▂ |  peak 16:00

Records
  longest commit streak  6 days (2026-11-09 .. 2026-11-14)
```

`--json` for the year: 65 KB, top-level keys `range`, `previous`, `totals`,
`comparisons`, `top`, `time_of_day`, `series` (365 values per metric) and
`records`.

## Decisions

- **Range math on `daily_metrics`, records on `sessions`.** Daily rows are
  already per local date, so every card is a range scan and a fold; only the
  records that need exact times read sessions.
- **Mean = mean of days with data.** A week with compressed memory sampled
  on 3 days averages those 3, not 3 days and 4 zeros. Recaps that want
  "per calendar day" can divide a `Sum` by `DateRange::days()`.
- **Percent change from zero is none, not infinity**; both zero is 0 %.
- **Latest night cut-off at 05:00** (`NIGHT_ENDS_HOUR`): a session from
  03:00 to 07:00 makes that night end at 05:00 and the next day's latest
  07:00.
- **Longest session is attributed to its start date** and not clipped to the
  range.
- **Streaks use screen time > 0 and commits > 0** as stored; a day with a
  row of 0 breaks a streak like a missing day.
- **JSON numbers**: whole values print without a fraction.

## Known limitations

- Latest night and longest session read `app_usage` sessions only; time an
  app on the active-without-input list kept you "active" counts like any
  other app time.
- Comparisons cover metrics without keys; per-key changes (an app's time vs
  last week) need two `top` calls.
- A collector turned off hides its history too; turning it back on shows it
  again.

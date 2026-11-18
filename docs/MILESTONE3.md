# Milestone 3: Disk & Memory collector

The Disk & Memory collector from DESIGN.md §7: live, every 5 minutes, off by
default. Accepted differences from the design doc are in
[DEVIATIONS.md](DEVIATIONS.md).

## What was built

- **Platform probe** (`crates/core/src/probe.rs`): core defines a plain
  `DiskMemoryReading` (boot time, disk bytes read / written, swap-in /
  swap-out bytes, memory pressure level, compressed memory bytes) and a
  `SystemProbe` trait. The platform layer sets one on the engine
  (`Engine::set_probe`); collectors read it through `Sink::read_disk_memory`.
  Tests use fake counters; a platform without one (Windows stub) returns
  `None` and the collector records nothing.
- **macOS counters** (`crates/collector/src/platform/macos/counters.rs`), no
  permission, no entitlement, no child process:
  - disk: IOKit `IOBlockStorageDriver` → `Statistics` → `Bytes (Read)` /
    `Bytes (Write)`, summed over drivers whose block storage device reports
    `Physical Interconnect Location = Internal`, excluding the built-in SD
    card reader (`Physical Interconnect = Secure Digital`). Disk images
    (location `File` / `Virtual`) and external drives are excluded. APFS
    containers sit above the physical drivers, so nothing counts twice;
  - swap and compressed memory: `host_statistics64(HOST_VM_INFO64)`
    `swapins` / `swapouts` / `compressor_page_count` × host page size (what
    `vm_stat` prints; "compressed" is the memory the compressor occupies, as
    Activity Monitor shows it);
  - pressure: `sysctl kern.memorystatus_vm_pressure_level` (1 / 2 / 4 →
    normal / warning / critical); boot time: `sysctl kern.boottime`.
- **Collector `disk_memory`** (`crates/core/src/collectors/disk_memory.rs`):
  - reads on the first minute tick at least 4.5 min after the previous
    reading (ticks are ~60 s ±5 s, so every 5th tick), only while awake and
    not paused. Dark-wake ticks (the engine's M1 asleep state, now shared
    with collectors) never read. The first tick after start takes a baseline;
  - each reading stores the deltas since the previous one, the pressure
    level and compressed memory at that moment, and the **awake seconds** it
    covers;
  - **resets**: a boot time more than 1 s different, any counter lower than
    before, a rate above 32 GiB/s, or the wall clock moving backwards
    re-baselines with no sample: never a negative or a huge delta;
  - **sleep**: the first awake tick after a wake records one delta for
    everything since the last reading (the last minutes before sleep plus
    dark-wake I/O), at the wake tick; its awake seconds are the time before
    the sleep plus the time since the wake, not the night;
  - **pause** drops the baseline; the first tick after resuming takes a new
    one, so nothing done while paused is recorded. **Deleting** the data
    re-baselines (new `Collector::on_data_deleted` hook).
- **Storage**: one `samples` row per reading, metric `reading`, value = awake
  seconds, the rest in the new `samples.data` column (schema v3) as JSON with
  short keys: `{"r":3153920,"w":13234176,"c":1630486528,"p":"normal"}`
  (`si` / `so` for swap, left out when zero).
- **Daily metrics** (only days a sample's window touches):
  `disk_memory/bytes_read`, `bytes_written`, `swap_in_bytes`,
  `swap_out_bytes`, `pressure_secs_by_level` (key `normal|warning|critical`),
  `compressed_peak_bytes`, `compressed_avg_bytes` (time-weighted),
  `bytes_written_by_hour` (key `00`..`23`). A reading at `t` covering `s`
  awake seconds describes the window `[t − s, t]` in its own UTC offset;
  bytes and pressure seconds are split across local days and hours in
  proportion to time. Pressure seconds therefore add up to sampled time.
- **CLI**: `wrapped-collector probe disk-memory` prints two readings 60 s
  apart and their deltas. Read-only: it opens no database. `stats` shows byte
  metrics in B / KB / MB / GB.

## Running it

```sh
export WRAPPED_DB=~/Documents/wrapped-notes/m3-scratch.db   # any test database
wrapped-collector probe disk-memory
wrapped-collector settings set collector.disk_memory.enabled true
wrapped-collector --debug            # samples every 5 min while awake
wrapped-collector stats today
```

## Decisions

- **Probe through the sink, not a new event.** The collector decides when to
  read (its own 5-minute cadence, asleep state, pause), so the platform does
  no reads nobody asked for, and tests drive it with fake counters.
- **Boot time decides resets, the counters only back it up.** A new boot
  whose counters are already above the old ones would otherwise look like a
  huge delta. The 1 s tolerance absorbs the boot time shifting when the wall
  clock is adjusted. A process cannot outlive a reboot, so in practice this
  guards against clock steps and driver resets; the baseline is in memory,
  and time while the collector is not running is not recorded (same as M1).
- **Awake seconds, not wall seconds.** A reading after a 9-hour sleep covers
  ~1–5 minutes of awake time; all its bytes land in that window at the wake
  tick. Spreading them over the night would invent activity at 3 a.m.
- **One row per reading** (schema v3), justified by the write budget below:
  as seven rows per reading (first version) each flush dirtied ~7 more 4 KB
  pages, one per metric's part of the `(collector, metric, time)` B-tree.
- **Instant values on both days.** A window crossing midnight counts its
  compressed-memory reading toward both days' peak and average.

## Measurements (release build, 2026-11-16)

### Write budget

`cargo test -p wrapped-core --release --test write_budget -- --ignored --nocapture`
(simulated day as in M1: app switch every 3 min, 20-min idle break every 4 h,
10-min flushes; fake counters; then a full checkpoint).

| Run | Samples | WAL bytes | Frames / flush | Checkpoint | Total / day | Kernel counter |
|---|---|---|---|---|---|---|
| 16 h, collector off | 0 | 1,017,672 | 2.5 | 36,864 | **1,054,536 (1.01 MB)** | 1,081,344 |
| 16 h, on, 7 rows per reading | 191 | 3,951,112 | 9.9 | 110,592 | 4,061,704 (3.87 MB) | 4,083,712 |
| 16 h, on, 1 row (long keys) | 191 | 1,730,432 | 4.3 | 73,728 | 1,804,160 (1.72 MB) | 1,826,816 |
| 16 h, on, 1 row (short keys) | 191 | 1,648,032 | 4.1 | 65,536 | 1,713,568 (1.63 MB) | 1,736,704 |
| 16 h, on, 1 row (short keys, integers) | 191 | 1,619,192 | 4.1 | 61,440 | **1,680,632 (1.60 MB)** | 1,703,936 |
| 24 h, collector off | 0 | 1,606,832 | 2.7 | 45,056 | **1,651,888 (1.58 MB)** | 1,679,360 |
| 24 h, on, 7 rows per reading | 287 | 7,041,112 | 11.8 | 163,840 | 7,204,952 (6.87 MB) | 7,229,440 |
| 24 h, on, 1 row (long keys) | 287 | 2,859,312 | 4.8 | 106,496 | 2,965,808 (2.83 MB) | 2,990,080 |
| 24 h, on, 1 row (short keys) | 287 | 2,743,952 | 4.6 | 94,208 | 2,838,160 (2.71 MB) | 2,859,008 |
| 24 h, on, 1 row (short keys, integers) | 287 | 2,673,912 | 4.5 | 90,112 | **2,764,024 (2.64 MB)** | 2,785,280 |

(287 / 191 samples: the first tick takes the baseline.)

- A normal 16-hour day stays within the 1–2 MB budget (1.60 MB).
- A machine **awake 24 h** with the collector on writes 2.64 MB/day, over
  the ~2 MB guideline; 1.58 MB of that is the M1 collectors alone over 24 h.
  **Accepted** (see DEVIATIONS.md, Disk & Memory); revisit at Milestone 9.
- Where the remaining +1.06 MB/24 h goes, measured: turning the
  disk_memory rollup off changed nothing for 16 h (1.72 → 1.72 MB with long
  keys), so it is not the daily metrics. Frames per flush were 3 for ~60 %
  of flushes (the appended `samples` leaf) and 6–8 for ~30 % (rebalancing
  when the rightmost leaf of the WITHOUT ROWID table fills). Shorter rows
  cut the rebalances a little (−0.19 MB with short keys and integers); the one leaf page per 10-minute
  flush (~0.6 MB/day at 145 flushes) remains. Not done (would change the
  M1 crash budget): holding disk_memory samples across several flushes.

### Probe

```
$ wrapped-collector probe disk-memory
reading 1 at 2026-11-16T21:57:12+08:00
  boot time              2026-11-16T17:23:00+08:00
  disk bytes read             15755665408  15.76 GB
  disk bytes written           7227486208  7.23 GB
  swap in bytes                         0  0 B
  swap out bytes                        0  0 B
  compressed bytes             1631649792  1.63 GB
  memory pressure                  normal
waiting 60 s ...
reading 2 at 2026-11-16T21:58:12+08:00
  boot time              2026-11-16T17:23:00+08:00
  disk bytes read             15758819328  15.76 GB
  disk bytes written           7240720384  7.24 GB
  swap in bytes                         0  0 B
  swap out bytes                        0  0 B
  compressed bytes             1630486528  1.63 GB
  memory pressure                  normal
deltas over 60.0 s
  disk bytes read                 3153920  3.15 MB
  disk bytes written             13234176  13.23 MB
  swap in bytes                         0  0 B
  swap out bytes                        0  0 B
  compressed bytes               -1163264
```

Cross-check, run back to back: `ioreg -r -c IOBlockStorageDriver` showed
`Bytes (Read)` 15,759,134,720 and `Bytes (Write)` 7,254,618,112 for the
internal NVMe driver (0 for the SD reader), and `vm_stat` 99,506 pages
occupied by the compressor (× 16,384 = 1,630,306,304); the probe printed
exactly those three values.

### Live run (release binary on a scratch database, 2026-11-16)

The bare release binary (not `Wrapped.app`) ran 16 minutes with `--debug`
on `m3-scratch.db`, then SIGTERM:

- baseline at start (21:57:06); readings at 22:02:11, 22:07:11, 22:12:11
  (awake seconds 304.989, 300.009, 299.990); minute tick gaps 59.9–65.0 s;
- flushes: 22:07:11 (2 samples), shutdown 22:13:06 (1 sample);
- daily metrics: `pressure_secs_by_level/normal` 905 s (= the three
  readings' seconds); the first window (21:57:06–22:02:11) split between
  `bytes_written_by_hour` 21 (83,622,914) and 22, in proportion to time;
- the last window shows 2.9 GB written: release builds and the stats
  benchmark running at the time, which is what the counter measures.

## Known limitations

- **Removable internal readers**: only the SD card reader is excluded by
  name (`Secure Digital`); another internal reader with removable media
  would be counted.
- **A drive appearing** after boot (none expected for internal storage) would
  add its counters since boot to one delta. A drive disappearing lowers the
  sum and re-baselines.
- **Time while the collector is not running** (quit, installing an update)
  is not recorded; the first reading after a start is only a baseline.
- **A failed read** keeps the old baseline; the next successful read covers
  the whole gap (capped at one hour of window, as any sample).
- **Pressure** is the level at the reading, credited to the 5 minutes before
  it; a short warning spike between readings is not seen.
- **24 h awake** exceeds the ~2 MB/day guideline (2.64 MB; see above).
  Accepted for now, revisit at Milestone 9 (DEVIATIONS.md, Disk & Memory).
- **Hour attribution after sleep**: I/O during sleep and dark wakes (Time
  Machine, Spotlight and other maintenance) lands in the hour of the wake
  tick, so `bytes_written_by_hour` shows a spike at wake that is not the
  user's own activity. The dashboard and recap cards (Milestones 5 and 8)
  should label disk activity as including background system activity.
- **A cancelled shutdown** (`WillPowerOff` then nothing) leaves the awake
  stretch closed until the next tick: under a minute of awake time is not
  counted in the next sample.
- **Zero swap is stored**: `swap_in_bytes` / `swap_out_bytes` are written
  (as 0) for every day with samples, so "no swap that day" is distinguishable
  from "not measured".
- **Windows**: stub only (`None`); Milestone 9.

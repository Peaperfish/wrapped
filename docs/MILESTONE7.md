# Milestone 7: the Now tab

DESIGN.md §15 step 7: live hardware numbers in the dashboard, next to the
Overview. Nothing is recorded: the numbers exist only while the Now tab is
shown, and the 60 s of history behind the sparklines lives in memory.

Status: **done**.

## What was built

- `crates/core/src/now/`: a `NowSampler` that does work only when `sample()` is
  called (no thread, no timer). It keeps the previous raw counters, so rates
  (CPU %, disk, network, swap) come from the delta between two calls; the first
  call returns every rate as `None`. Two calls closer than 100 ms are not turned
  into rates. Every field of `NowSnapshot` is an `Option`: a metric the machine
  does not expose is `None` and shown as "—". Core stays free of serde and
  ts-rs; the JSON comes from `NowSnapshot::to_json`.
- `wrapped-collector now [--json]`: prints one sample (after a short baseline).
- Dashboard: the `now_snapshot` command (typed `NowSnapshotJson`, exported to
  TS with ts-rs in the dashboard crate) and the **Overview | Now** switch. The
  last tab is remembered in localStorage.
- Cards: CPU (ring, P / E split, per-core bars by cluster, sparkline), Memory,
  GPU, Disk, Network, Temperatures, Fans. A card with only `None` values is
  hidden; a single `None` shows "—". When every value is `None` after two
  samples (Windows for now) the tab says "Live hardware stats aren't available
  on this platform yet".

## Sources per metric (macOS, public APIs only, no extra crates)

| Metric | Source |
|---|---|
| CPU per core | `host_processor_info(PROCESSOR_CPU_LOAD_INFO)`; P / E cluster from IORegistry `IODeviceTree:/cpus/cpuN` → `cluster-type`, read once |
| GPU | IOKit `IOAccelerator` → `PerformanceStatistics` → `Device Utilization %` |
| Memory | `host_statistics64(HOST_VM_INFO64)`: used = app + wired + compressed (Activity Monitor's definition); `hw.memsize`; `kern.memorystatus_vm_pressure_level` (1 normal, 2 warning, 4 critical) |
| Swap | size from `vm.swapusage`; in / out counters from the VM statistics |
| Disk | IOKit `IOBlockStorageDriver` statistics of internal drives (as the Disk & Memory collector) |
| Network | `sysctl NET_RT_IFLIST2` (64-bit byte counters per interface) |
| Temperatures, fans | AppleSMC IOKit user client (`T…`, `F…` keys); only keys with a confident label are reported |

Units: memory is 1024-based GB (as Activity Monitor); disk, network and swap
rates are decimal (kB/s, MB/s, GB/s), as in the CLI.

## The P-core sleep rule

The performance-core temperature keys (`Tp1x` / `Tp2x`) read 0-8 °C while the
cluster is power-gated and 58-74 °C under load (measured on a Mac14,9 with 8
busy processes). A reading that low is not a temperature, so the P-core group
is `None` while the cluster sleeps. The UI shows **"idle"** for it when the
E-core value exists, and "—" otherwise. Other `Tp…` keys are not cores and are
left out, as is every key without a known meaning: a group may be missing on
another chip, never wrong.

## Costs

- One sample: median **6.7 ms**, worst **~28 ms**.
- `now_snapshot` is an `async` command that runs the sample on a blocking
  worker (`spawn_blocking`), so the UI thread never waits for it.
- Polling: once per second, one call at a time, only while the Now tab is shown
  and the window is visible. On any other tab, or hidden, no timer exists and
  the history is cleared.
- Measured on the Now tab: dashboard process **~1.3 %** + web content
  **~2.3 %** of one core.
- Bundle (`ui/dist/assets`): JS 120.53 → 132.97 kB (gzip 42.44 → 45.93), CSS
  27.64 → 29.91 kB (gzip 5.94 → 6.39).
- Binaries: collector 4,339,904 → 4,446,880 B; dashboard 14,099,968 →
  14,228,592 B.

## Not available

- Windows: every value is `None` until the Windows sources arrive (milestone 9).
- Fanless Macs: no Fans card. Chips whose SMC keys differ: missing temperature
  groups.

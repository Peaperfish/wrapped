# Spikes

## m0 — front app + idle + sleep/wake + display sleep (macOS, `spikes/spike`)

Run: `cd spikes/spike && cargo run --release`

### Crates
`objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-core-graphics`, `block2`, `libc` (timestamps, `proc_name`). IOKit is linked directly (`IOPMCopyAssertionsByProcess`, public API).

### Behaviour
- Front app via `NSWorkspaceDidActivateApplicationNotification`; printed only when the bundle id differs from the previous one.
- Every line is stamped UTC plus the local offset, e.g. `2026-11-16T06:20:47Z +08:00`.
- Every 60 s (run-loop `NSTimer`): idle seconds via `CGEventSourceSecondsSinceLastEventType` (idle at >=180 s) and whether any process holds `PreventUserIdleDisplaySleep`, via `IOPMCopyAssertionsByProcess`. No permission prompt appeared for either.
- Observers for willSleep / didWake / screensDidSleep / screensDidWake; a "sleep-cycle events" line on wake lists which fired.

### Results (live run, 06:20:47Z to 06:39:54Z, ~19 min)
- Tests a-d all behaved as expected (below).
- **Lid close (MacBook) produced `willSleep` + `didWake` only. `screensDidSleep` / `screensDidWake` did not fire** in that cycle. Display-only sleep (screen off, system awake) was not tested, so those two observers are unconfirmed.
- `loginwindow` appeared as the front app at sleep, then Ghostty on wake.
- Caffeinate (`caffeinate -d`) detected as `caffeinate[pid] "caffeinate command-line tool"` and cleared after it exited.
- Idle: counter climbed 76 -> 316 s untouched; `idle` printed from 196 s.

### Measurements (release binary, pid 25753)
| Metric | Start (06:20:49Z) | ~5 min (06:26:50Z) | End (06:39:54Z) |
|---|---|---|---|
| RSS (KB) | 11936 | 12080 | 12224 |
| CPU time (`ps -o time=`) | 0:00.03 | 0:00.04 | 0:00.06 |

CPU time is 0.06 s total over ~19 min (`ps` resolution is 0.01 s). `top -l 2 -s 5 -stats pid,command,cpu,idlew,power -pid PID`, run twice (second sample of each): %CPU 0.0, IDLEW (idle wakeups) 0, POWER 0.0 both times. These are 5 s snapshots, so they don't capture the once-a-minute tick; the CPU-time total does. Activity Monitor Energy tab and battery-unplugged behaviour were not measured. RSS grew ~290 KB over the run, which is too small a trend to call a leak.

### Not verified
- Display-only sleep notifications (see above).
- Sleep via Apple menu (only lid close tested).
- Anything on battery.

### Notes
- The 60 s timer may be coalesced by the OS; ticks were exactly 60 s apart here.
- Callbacks run on the main run loop; no extra threads.

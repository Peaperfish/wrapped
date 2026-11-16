# Spikes

## m0 — front app + idle + sleep/wake (macOS, `spikes/spike`)

Run: `cd spikes/spike && cargo run --release`

### Crates
`objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-core-graphics`, `block2` (default features).

### What worked
- Front-app changes via `NSWorkspaceDidActivateApplicationNotification` on `NSWorkspace.notificationCenter`; prints name + bundle id. Event-driven, no polling. Verified live (Ghostty <-> Chrome switches).
- Idle via `CGEventSourceSecondsSinceLastEventType(CombinedSessionState, kCGAnyInputEventType)`, printed once a minute from an `NSTimer` on the run loop. Verified the seconds value tracks input; no permission prompts appeared.
- No Accessibility / Input Monitoring / Screen Recording needed for any of it.
- Measured over ~2 min (`ps -o %cpu,rss`, 5 samples): 0.0% CPU, RSS 10.1–10.3 MB (budget: <20 MB). `ps` %cpu is a lifetime average, so it is coarse; Activity Monitor Energy was not checked.

### Not verified
- Sleep/wake lines (`willSleep` / `didWake`) are registered but were **not triggered** during the run, so untested.
- The `IDLE` branch (>=180 s) was never hit; only the "active" path was seen.
- Display sleep / lid close are not handled separately (design doc §13 says they should end a session); only system sleep/wake is observed.
- Energy impact on battery not measured.

### Notes
- The 60 s `NSTimer` may be coalesced by the OS (timer tolerance), so ticks drift slightly (~60–62 s seen).
- Notification callbacks run on the main thread's run loop; no extra threads.

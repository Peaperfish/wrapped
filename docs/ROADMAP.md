# Wrapped roadmap

## 0.1.1 · Release documentation and startup fixes

Document the 0.1.0 downloads, requirements, first launch, and platform support. Fix installation and startup issues that can be reproduced.

## 0.2.0 · Previous-period comparisons on Overview

Add an optional comparison for **This week** and **This month**, using the previous calendar week or month as the baseline. Show the date ranges, active time, daily average, and the apps with the largest gains or losses in active time.

- For an incomplete period, compare the same elapsed duration from the start of each period. If the previous period is shorter than the elapsed duration, cap both ranges at its length. Use the duration of each displayed range to calculate its daily average.
- If either range lacks sufficient recorded history, show **Insufficient history** and omit the change for the affected metric. Keep any available current totals visible. A gap in recorded history is distinct from a recorded zero.
- For a recorded zero baseline, show the absolute change and use a dash for the percentage. If both periods recorded zero, the absolute change is zero.

## 0.3.0 · Comparison slide in recaps

Add a previous-period comparison slide to weekly and monthly recaps. Reuse the Overview comparison logic, including elapsed-range alignment and the treatment of missing history and zero baselines. Preserve 1080 × 1920 portrait PNG export.

## Later

- Use Developer ID signing and notarization for macOS releases.
- Add Windows Drive Health when there is a practical approach that lets the collector keep running without administrator privileges.

## Project history

The original milestones through the first release, [Wrapped 0.1.0](https://github.com/Peaperfish/wrapped/releases/tag/v0.1.0).

- [x] **0.** [Spike](../spikes/README.md) for foreground app, idle state, and sleep/wake
- [x] **1.** [Collector](MILESTONE1.md), plugin system, SQLite, daily metrics, tracking settings, tray, and launch at login
- [x] **2.** [Git and Photos collectors](MILESTONE2.md)
- [x] **3.** [Disk and Memory collector](MILESTONE3.md)
- [x] **4.** [Stats engine](MILESTONE4.md) for any date range
- [x] **5.** [Dashboard](MILESTONE5.md) with heatmap and cards
- [x] **6.** [Onboarding and Settings](MILESTONE6.md)
- [x] **7.** [Now tab](MILESTONE7.md)
- [x] **8.** [Recap slides and notifications](MILESTONE8.md)
- [x] **9.** [Windows x64 port](MILESTONE9.md)
- **10.** Drive Health
  - [x] macOS collector shipped in 0.1.0
  - [ ] Windows collector deferred

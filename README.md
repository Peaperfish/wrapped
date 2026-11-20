# Wrapped

A local-only desktop app that tracks how you use your computer and turns it into a dashboard, a live hardware tab, and Spotify-Wrapped-style recaps.

> **Status: early development.** The collector runs on macOS (tray icon): app usage, active / idle, git, photos and disk & memory, plus a stats engine for any date range and a first dashboard window (year heatmap, range presets; cards to come); see [docs/MILESTONE1.md](docs/MILESTONE1.md) to [docs/MILESTONE5.md](docs/MILESTONE5.md).

*Wrapped is a working name and may change. This project is not affiliated with Spotify.*

<!-- TODO: screenshots (dashboard heatmap, Now tab, recap slides) once the UI exists. -->

## Features (planned for v1)

- **Dashboard** for any date range, with a year heatmap.
- **Now tab** with live hardware stats.
- **Weekly, monthly and yearly recaps** as story slides.
- **Collectors** (each one opt-in and can be turned off or deleted at any time):
  - App usage: front app plus start/end times (on by default)
  - Active / idle: away periods (on by default)
  - Git: commit times and repo names for folders you pick (off by default)
  - Photos: EXIF only (date, camera, lens, settings) for folders you pick (off by default)
  - Disk and memory: disk GB read/written, swap, memory pressure (off by default)

See [docs/DESIGN.md](docs/DESIGN.md) for the full design.

## Privacy

**Everything stays on your machine.**

- Local-only: no accounts, no cloud, no uploading.
- **No network access** by the collector or the app.
- **No telemetry**, analytics or crash reporting.

**Never tracked:**

- Keystrokes, neither content nor counts. No keyboard hooks of any kind.
- Clipboard contents
- Screenshots
- URLs
- File contents

You can pause tracking from the tray, exclude apps, export everything, or delete everything.

## Performance budgets

| Area | Budget |
| :-- | :-- |
| Battery (Mac, unplugged) | Energy impact ~0 |
| Gaming (Windows) | No FPS impact |
| RAM (collector) | < 20 MB |
| SSD writes | ~1-2 MB/day |

The collector is a plain Rust process with a native tray icon (no web view) and is event-driven rather than polling. The dashboard is a separate Tauri window that only runs while open.

## Platform support

- macOS: in development (the spike runs here).
- Windows x64: collector, tray and dashboard implemented. Native verification is pending. See [Windows setup and validation](docs/MILESTONE9.md).

## Roadmap

- [x] **0.** Spike: front app, idle state, sleep/wake (see [spikes/README.md](spikes/README.md))
- [x] **1.** Collector, plugin system, SQLite, daily metrics, tracking settings, tray, launch at login (**next**)
- [x] **2.** Collectors: Git and Photos
- [x] **3.** Collector: Disk and Memory
- [x] **4.** Stats engine (any date range)
- [x] **5.** Dashboard: heatmap and cards
- [x] **6.** Onboarding and Settings
- [x] **7.** Now tab
- [x] **8.** Recap slides and notifications
- [ ] **9.** Windows port
- [ ] **10.** Collector: Drive Health

## Build from source

For Windows, see the [PowerShell build and launch instructions](docs/MILESTONE9.md). The macOS commands below require Rust stable (via rustup) and macOS 13+.

```sh
cargo build --release -p wrapped-collector
./target/release/wrapped-collector            # tray icon; records to ~/Library/Application Support/Wrapped
./target/release/wrapped-collector help       # temporary CLI: settings, stats, sessions, export
```

To install with launch at login, see [docs/MILESTONE1.md](docs/MILESTONE1.md#install-with-launch-at-login).

The dashboard (`dashboard/`, Tauri + Svelte) also needs Node.js 20.19+; see [docs/MILESTONE5.md](docs/MILESTONE5.md#develop).

To try the Milestone 0 spike (macOS only), see [spikes/README.md](spikes/README.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Please also read the [Code of Conduct](CODE_OF_CONDUCT.md) and [Security policy](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

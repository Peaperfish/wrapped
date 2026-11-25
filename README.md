# Wrapped

Wrapped is a local-only desktop app that shows how you spend time on your computer, displays live hardware stats, and turns your activity into recaps.

**Download [Wrapped 0.1.0](https://github.com/Peaperfish/wrapped/releases/tag/v0.1.0).** Choose the ZIP for your operating system and processor. The release page also includes SHA-256 checksums.

## Download and launch

| Platform | Download | Requirements |
| --- | --- | --- |
| macOS | `wrapped-0.1.0-macos-arm64.zip` | Apple Silicon (M-series) and macOS 13 or later |
| Windows | `wrapped-0.1.0-windows-x64.zip` | Windows 10 or 11 on x64 and the Microsoft Edge WebView2 Runtime |

**macOS.** Extract the ZIP, move `Wrapped.app` to `~/Applications`, and open it from Finder. Use the Wrapped menu bar icon to open the dashboard. The app is ad hoc signed and the release has no notarization.

**Windows.** Extract the entire ZIP into a folder you plan to keep, such as `%LOCALAPPDATA%\Programs\Wrapped`. Keep the extracted files together and run `Wrapped.exe`. Use the Wrapped icon in the system tray to reopen the dashboard.

On first launch, choose which collectors to enable in the setup checklist, then select **Start recording**.

## What shipped in 0.1.0

- **Overview** shows screen time, top apps, and daily activity. Pick a preset or custom date range and explore the year heatmap.
- **Now** shows live CPU, memory, disk, network, and other readings exposed by your hardware. The live readings stay in memory while the tab is visible.
- **Recaps** turns weekly, monthly, and yearly activity into story slides. Export individual slides as 1080 × 1920 portrait PNGs or opt in to recap notifications in Settings.
- **Drive Health** is an opt-in macOS collector for supported internal NVMe drives. It uses an existing `smartctl` installation from smartmontools.

The Windows build and smoke tests passed in native Windows CI, and 0.1.0 passed a manual check in Parallels. Brian Wang also completed a check on a physical Windows PC. Windows Drive Health remains deferred. See [Windows setup and validation](docs/MILESTONE9.md).

## Privacy

Activity data is stored in a SQLite database on your device. Wrapped has no accounts, cloud sync, uploads, or telemetry. It does not record keystrokes, clipboard contents, screenshots, or URLs.

Choose what to collect during setup or in Settings. You can pause collection from the menu bar or system tray, exclude apps, and export your data as JSON or CSV or delete it from Settings.

## Roadmap

See the [roadmap and project history](docs/ROADMAP.md) for the proposed releases and original milestones. For background on the project, see the [design notes](docs/DESIGN.md).

## Build from source

For Windows, see the [PowerShell build and launch instructions](docs/MILESTONE9.md#build-and-launch). The macOS commands below require Rust stable (via rustup) and macOS 13+.

```sh
cargo build --release -p wrapped-collector
./target/release/wrapped-collector            # tray icon; records to ~/Library/Application Support/Wrapped
./target/release/wrapped-collector help       # CLI: settings, stats, sessions, export
```

To install with launch at login, see [the collector setup instructions](docs/MILESTONE1.md#install-with-launch-at-login).

The dashboard (`dashboard/`, Tauri + Svelte) also needs Node.js 20.19+. See the [dashboard development instructions](docs/MILESTONE5.md#develop).

To try the Milestone 0 spike (macOS only), see the [spike README](spikes/README.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Please also read the [Code of Conduct](CODE_OF_CONDUCT.md) and [Security policy](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/license/mit>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

# Contributing

Wrapped is at an early stage (see the roadmap in the [README](README.md)) and the architecture is still moving.

- **Open an issue before a big PR.** Small fixes are welcome directly.
- `cargo fmt` and `cargo clippy -- -D warnings` must pass; CI checks both on macOS and Windows.
- **The privacy rules are non-negotiable.** No network access, no telemetry, and never track keystrokes (content or counts, no keyboard hooks), clipboard, screenshots, URLs or file contents. New data collection must be opt-in and called out in the PR. See [docs/DESIGN.md](docs/DESIGN.md).
- Contributions are dual licensed MIT OR Apache-2.0, as described in the README.
- Be kind: see the [Code of Conduct](CODE_OF_CONDUCT.md).

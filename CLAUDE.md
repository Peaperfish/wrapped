# Wrapped: standing rules

- After any change that affects the collector binary, commit it, then finish by
  running `scripts/macos/install.sh`, so the installed
  `~/Applications/Wrapped.app` is always the latest committed build (a dirty
  tree still installs, with a warning and a `-dirty` version). It is the only Wrapped.app that should exist.
- Never run `install.sh --force` while a measurement run is going (it quits a
  collector running from elsewhere).
- Commit locally only. No `Co-Authored-By`, `Claude-Session` or
  "Generated with Claude Code" lines. Don't push.
- Run cargo with `-q` and show only failures.
- Leave `spikes/` unchanged.

## Windows-safe checklist (before every commit)

- Paths never go into JSON or strings by hand: use serde_json (`json!` /
  `to_string`), not `format!` with `display()`.
- Test paths come from `std::env::temp_dir()` or a temp folder, never `/tmp/...`
  or other Unix-absolute literals; join path parts with `.join()`, never `"a/b"`.
- Nothing depends on `HOME` alone (also `USERPROFILE`); `~` handling accepts
  `~/` and `~\`.
- Don't call `drop()` on values that may not implement Drop on every platform;
  use a block scope.
- Code behind `cfg(windows)` / `platform/unsupported.rs` only compiles in CI. If
  the `x86_64-pc-windows-gnu` target is installed, also run:
  `cargo clippy -q --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`

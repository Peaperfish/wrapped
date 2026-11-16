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

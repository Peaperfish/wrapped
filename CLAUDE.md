# Wrapped: standing rules

- After any change that affects the collector binary, finish by running
  `scripts/macos/install.sh`, so the installed `~/Applications/Wrapped.app` is
  always the latest build. It is the only Wrapped.app that should exist.
- Never run `install.sh --force` while a measurement run is going (it quits a
  collector running from elsewhere).
- Commit locally only. No `Co-Authored-By`, `Claude-Session` or
  "Generated with Claude Code" lines. Don't push.
- Run cargo with `-q` and show only failures.
- Leave `spikes/` unchanged.

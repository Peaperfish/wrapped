#!/bin/bash
# Build the release bundle and install it as the one and only Wrapped.app:
#   ~/Applications/Wrapped.app   (same path + bundle id every time, so the
#   SMAppService login item stays valid).
#
# Usage: scripts/macos/install.sh [--force]
#   --force  also replace a collector that is running from somewhere else
#            (e.g. a debug or measurement run from target/). Never use it while
#            a measurement run is going.
set -euo pipefail
cd "$(dirname "$0")/../.."
REPO=$(pwd)

FORCE=0
case "${1:-}" in
    "") ;;
    --force) FORCE=1 ;;
    *) echo "usage: $0 [--force]" >&2; exit 2 ;;
esac

DEST="$HOME/Applications/Wrapped.app"
DEST_EXE="$DEST/Contents/MacOS/wrapped-collector"
DB="${WRAPPED_DB:-$HOME/Library/Application Support/Wrapped/wrapped.db}"
LOCK="${DB%.*}.lock"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
PLIST=/usr/libexec/PlistBuddy

die() { echo "install: $*" >&2; exit 1; }

plist_get() { "$PLIST" -c "Print :$2" "$1/Contents/Info.plist" 2>/dev/null || echo "?"; }
app_version() { # "<short version> (<build>)" of a bundle
    echo "$(plist_get "$1" CFBundleShortVersionString) ($(plist_get "$1" CFBundleVersion))"
}
# Pid of the collector holding the single-instance lock, or nothing.
lock_pid() { [ -e "$LOCK" ] && lsof -t -- "$LOCK" 2>/dev/null | head -1 || true; }
exe_of() { ps -p "$1" -o comm= 2>/dev/null | sed 's/^ *//'; }

# 1. Build release + bundle (target/release/Wrapped.app).
scripts/macos/bundle.sh
BUILT="$REPO/target/release/Wrapped.app"
HEAD_HASH=$(git rev-parse --short HEAD)
NEW_VERSION=$(app_version "$BUILT")
OLD_VERSION="none"
[ -d "$DEST" ] && OLD_VERSION=$(app_version "$DEST")

# 2. Stop a running collector, gracefully.
PID=$(lock_pid)
if [ -n "$PID" ]; then
    EXE=$(exe_of "$PID")
    if [ "$EXE" != "$DEST_EXE" ] && [ "$FORCE" -ne 1 ]; then
        die "a collector (pid $PID) is running from $EXE, not from $DEST_EXE.
It may be a debug or measurement run, so I am not touching it. Stop it yourself,
or pass --force to quit it and install anyway."
    fi
    echo "stopping collector pid $PID (SIGTERM, it flushes before exiting)"
    kill -TERM "$PID"
    for _ in $(seq 1 100); do
        kill -0 "$PID" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$PID" 2>/dev/null; then
        die "collector pid $PID is still running 10 s after SIGTERM; not replacing the app"
    fi
fi

# 3. Replace the installed app: copy next to it under a temp name, then swap.
mkdir -p "$HOME/Applications"
TMP=$(mktemp -d "$HOME/Applications/.Wrapped-install.XXXXXX") # no .app suffix: never registered
ditto "$BUILT" "$TMP/Wrapped.app"
if [ -d "$DEST" ]; then
    # renamex_np(RENAME_SWAP): one atomic exchange, no moment without an app.
    python3 -I - "$TMP/Wrapped.app" "$DEST" <<'PY' || die "atomic swap failed"
import ctypes, os, sys
libc = ctypes.CDLL(None, use_errno=True)
new, dest = (os.fsencode(p) for p in sys.argv[1:3])
if libc.renamex_np(new, dest, 2) != 0:  # 2 = RENAME_SWAP
    e = ctypes.get_errno(); sys.exit(f"renamex_np: {os.strerror(e)}")
PY
else
    mv "$TMP/Wrapped.app" "$DEST"
fi
rm -rf "$TMP" # holds the previous app after a swap

# 4. Relaunch and verify.
open "$DEST"
PID=""
for _ in $(seq 1 100); do
    PID=$(lock_pid)
    [ -n "$PID" ] && break
    sleep 0.1
done
[ -n "$PID" ] || die "the new app did not start (no process holds $LOCK)"
[ "$(exe_of "$PID")" = "$DEST_EXE" ] || die "pid $PID is running $(exe_of "$PID"), expected $DEST_EXE"
RUNNING=$("$DEST_EXE" --version)
case "$RUNNING" in
    *"($HEAD_HASH"*) ;;
    *) die "version mismatch: installed binary says '$RUNNING', HEAD is $HEAD_HASH" ;;
esac
case "$RUNNING" in
    *-dirty*) echo "warning: built from a tree with uncommitted changes ($RUNNING)" >&2 ;;
esac

# 5. Remove every other Wrapped.app the build produced, unregistering first so
#    LaunchServices (and the login item) only know the installed copy.
#    Only app bundles are removed; no cargo clean.
REMOVED=0
while IFS= read -r app; do
    [ -n "$app" ] || continue
    "$LSREGISTER" -u "$app" || true
    rm -rf "$app"
    REMOVED=$((REMOVED + 1))
done < <(find "$REPO/target" -name 'Wrapped.app' -type d -prune)

echo "Wrapped.app: $OLD_VERSION -> $NEW_VERSION, installed at $DEST (pid $PID), removed $REMOVED other bundle(s)"

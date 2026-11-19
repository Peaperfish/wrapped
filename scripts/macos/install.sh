#!/bin/bash
# Build the release bundle and install it as the one and only Wrapped.app:
#   ~/Applications/Wrapped.app   (same path + bundle id every time, so the
#   SMAppService login item stays valid), with the dashboard nested at
#   Contents/Helpers/Wrapped Dashboard.app. Both are replaced in one atomic
#   swap and must report the same version.
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
DASH_REL="Contents/Helpers/Wrapped Dashboard.app"
DASH_EXE_REL="$DASH_REL/Contents/MacOS/wrapped-dashboard"
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
# Pid of the process whose executable is exactly $1 (anchored, dots escaped), not
# merely one that has the lock file open (mdworker_shared once matched).
pid_of_exe() {
    pgrep -f "^$(printf '%s' "$1" | sed 's/[][\.*^$+?(){}|]/\\&/g')( |\$)" 2>/dev/null | head -1 || true
}
exe_of() { ps -p "$1" -o comm= 2>/dev/null | sed 's/^ *//'; }

# 1. Build release + bundle (target/release/Wrapped.app with the dashboard nested).
scripts/macos/bundle.sh
BUILT="$REPO/target/release/Wrapped.app"
HEAD_HASH=$(git rev-parse --short HEAD)
NEW_VERSION=$(app_version "$BUILT")
[ -x "$BUILT/$DASH_EXE_REL" ] || die "the build has no dashboard at $DASH_REL"
NEW_DASH_VERSION=$(app_version "$BUILT/$DASH_REL")
[ "$NEW_DASH_VERSION" = "$NEW_VERSION" ] ||
    die "built versions differ: collector $NEW_VERSION, dashboard $NEW_DASH_VERSION"
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

# 2b. Quit a dashboard running from the installed app (it only reads, so a
#     plain SIGTERM loses nothing). It is not relaunched: it runs on demand.
DASH_PIDS=$(pgrep -f "^$DEST/$DASH_EXE_REL" || true)
for DPID in $DASH_PIDS; do
    echo "quitting dashboard pid $DPID"
    kill -TERM "$DPID" 2>/dev/null || true
done
for DPID in $DASH_PIDS; do
    for _ in $(seq 1 50); do
        kill -0 "$DPID" 2>/dev/null || break
        sleep 0.1
    done
done

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
    PID=$(pid_of_exe "$DEST_EXE")
    [ -n "$PID" ] && break
    sleep 0.1
done
[ -n "$PID" ] || die "the new app did not start (no process runs $DEST_EXE)"
[ "$(exe_of "$PID")" = "$DEST_EXE" ] || die "pid $PID is running $(exe_of "$PID"), expected $DEST_EXE"
RUNNING=$("$DEST_EXE" --version)
case "$RUNNING" in
    *"($HEAD_HASH"*) ;;
    *) die "version mismatch: installed binary says '$RUNNING', HEAD is $HEAD_HASH" ;;
esac
DASH_RUNNING=$("$DEST/$DASH_EXE_REL" --version)
case "$DASH_RUNNING" in
    *"($HEAD_HASH"*) ;;
    *) die "version mismatch: installed dashboard says '$DASH_RUNNING', HEAD is $HEAD_HASH" ;;
esac
[ "${RUNNING##* }" = "${DASH_RUNNING##* }" ] ||
    die "collector ($RUNNING) and dashboard ($DASH_RUNNING) builds differ"
[ "$(app_version "$DEST/$DASH_REL")" = "$(app_version "$DEST")" ] ||
    die "Info.plist versions differ: $(app_version "$DEST") vs dashboard $(app_version "$DEST/$DASH_REL")"
case "$RUNNING" in
    *-dirty*) echo "warning: built from a tree with uncommitted changes ($RUNNING)" >&2 ;;
esac

# 5. Remove every other Wrapped.app the build produced (target/release/Wrapped.app included), unregistering first so
#    LaunchServices (and the login item) only know the installed copy.
#    Only app bundles are removed; no cargo clean.
REMOVED=0
while IFS= read -r app; do
    [ -n "$app" ] || continue
    [ -d "$app/$DASH_REL" ] && { "$LSREGISTER" -u "$app/$DASH_REL" || true; }
    "$LSREGISTER" -u "$app" || true
    rm -rf "$app"
    REMOVED=$((REMOVED + 1))
done < <(find "$REPO/target" -name 'Wrapped.app' -type d -prune)

# 6. Unregister every other copy Launch Services still knows about (deleted
#    build folders, moved apps), for both bundle ids. Only the registration is
#    removed here; files outside target/ are never deleted.
ID_APP=io.github.peaperfish.wrapped
ID_DASH=io.github.peaperfish.wrapped.dashboard
STALE=0
while IFS= read -r path; do
    [ -n "$path" ] || continue
    case "$path" in
        "$DEST"|"$DEST/$DASH_REL") continue ;;
    esac
    "$LSREGISTER" -u "$path" 2>/dev/null || true
    STALE=$((STALE + 1))
done < <("$LSREGISTER" -dump 2>/dev/null | awk -v a="$ID_APP" -v d="$ID_DASH" '
    /^path:/ { p = $0; sub(/^path: +/, "", p); sub(/ \(0x[0-9a-f]+\)$/, "", p) }
    /^identifier:/ { id = $2; if (id == a || id == d) print p }' | sort -u)

echo "Wrapped.app: $OLD_VERSION -> $NEW_VERSION, installed at $DEST (pid $PID), dashboard $DASH_RUNNING, removed $REMOVED other bundle(s), unregistered $STALE stale path(s)"

# 7. What Spotlight knows now: exactly the installed copies.
sleep 1
for ID in "$ID_APP" "$ID_DASH"; do
    FOUND=$(mdfind "kMDItemCFBundleIdentifier == '$ID'" || true)
    echo "mdfind $ID:"
    echo "${FOUND:-  (none)}"
    EXPECT="$DEST"
    [ "$ID" = "$ID_DASH" ] && EXPECT="$DEST/$DASH_REL"
    OTHERS=$(printf '%s\n' "$FOUND" | grep -v -x -F -e "$EXPECT" -e '' || true)
    [ -z "$OTHERS" ] || echo "warning: other copies of $ID are still indexed:
$OTHERS" >&2
done

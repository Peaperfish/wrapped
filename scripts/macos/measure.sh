#!/bin/sh
# Print one line of collector measurements, for the Milestone 1 overnight run.
#
#   scripts/macos/measure.sh [pid] [debug-log]
#
# Defaults: pid from ~/wrapped-phase-c/pid, log ~/wrapped-phase-c/debug.log.
# Prints: local time, elapsed, RSS (ps, KB), physical footprint (vmmap; the
# figure Activity Monitor's Memory column shows), total CPU time, sizes in
# bytes of the database, WAL, shared-memory index and debug log, and tick-gap
# stats from the debug log (gaps across a sleep/wake are excluded).
# Bytes written by the process is not available from the command line: read
# it from Activity Monitor → Disk → Bytes Written.
set -eu

PID=${1:-$(cat "$HOME/wrapped-phase-c/pid")}
LOG=${2:-$HOME/wrapped-phase-c/debug.log}
DIR="$HOME/Library/Application Support/Wrapped"

if ! ps -p "$PID" >/dev/null 2>&1; then
    echo "process $PID is not running" >&2
    exit 1
fi

size() { stat -f %z "$1" 2>/dev/null || echo 0; }
footprint=$(vmmap --summary "$PID" 2>/dev/null | awk '/^Physical footprint:/ {print $3}')

gaps=$(grep 'tick gap' "$LOG" 2>/dev/null | grep -v 'sleep/wake' | awk '{print $4}' | tr -d 's' || true)
if [ -n "$gaps" ]; then
    n=$(echo "$gaps" | wc -l | tr -d ' ')
    max=$(echo "$gaps" | sort -n | tail -1)
    over90=$(echo "$gaps" | awk '$1 > 90' | wc -l | tr -d ' ')
    gapinfo="ticks=$n max_gap_s=$max gaps_over_90s=$over90"
else
    gapinfo="ticks=0 (no debug log?)"
fi
warnings=$(grep -c WARNING "$LOG" 2>/dev/null || true)

echo "$(date '+%Y-%m-%dT%H:%M:%S%z') pid=$PID" \
    "etime=$(ps -o etime= -p "$PID" | tr -d ' ')" \
    "rss_kb=$(ps -o rss= -p "$PID" | tr -d ' ')" \
    "footprint=${footprint:-?}" \
    "cpu=$(ps -o time= -p "$PID" | tr -d ' ')" \
    "db=$(size "$DIR/wrapped.db")" \
    "wal=$(size "$DIR/wrapped.db-wal")" \
    "shm=$(size "$DIR/wrapped.db-shm")" \
    "debug_log=$(size "$LOG")" \
    "$gapinfo warnings=${warnings:-0}"

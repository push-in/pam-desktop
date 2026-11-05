#!/usr/bin/env bash
# Runs an example on a private virtual display and captures the window.
#
#   scripts/shot.sh <app dir> <output.png> [WIDTHxHEIGHT] [seconds] [-- xdotool arguments…]
#
# Used for visual checks during development; nothing is shown on the real screen.
set -euo pipefail

app=${1:?app directory}
output=${2:?output png}
size=${3:-960x640}
seconds=${4:-4}
shift $(( $# < 4 ? $# : 4 ))
[ "${1:-}" = "--" ] && shift

root=$(cd "$(dirname "$0")/.." && pwd)
display=${PAM_SHOT_DISPLAY:-:99}
if ! xdpyinfo -display "$display" >/dev/null 2>&1; then
    Xvfb "$display" -screen 0 1600x1200x24 -nolisten tcp >/dev/null 2>&1 &
    sleep 1
fi
export DISPLAY=$display
log=$(mktemp)
# Every capture starts from a clean application state.
state=$(mktemp -d)
export PAM_DESKTOP_STATE_DIR=$state PAM_DESKTOP_DATA_DIR=$state/data PAM_DESKTOP_CACHE_DIR=$state/cache
PAM_DESKTOP_WINDOW=$size timeout $(( seconds + 30 )) "$root/target/debug/pam-desktop" run "$app" >"$log" 2>&1 &
host=$!
sleep "$seconds"
if [ "$#" -gt 0 ]; then
    xdotool "$@"
    sleep 1
fi
import -window root -crop "${size}+0+0" +repage "$output"
kill "$host" 2>/dev/null || true
wait "$host" 2>/dev/null || true
grep -v 'DRI3\|MESA\|Note: you can' "$log" | head -40 || true
rm -rf "$log" "$state"

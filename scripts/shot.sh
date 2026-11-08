#!/usr/bin/env bash
# Runs an example on a private virtual display and captures the window.
#
#   scripts/shot.sh <app dir> <output.png> [WIDTHxHEIGHT] [seconds] [-- xdotool arguments… [then more…]]
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
# File dialogs and settings must not go through the desktop portal of the
# real session: everything stays on the virtual display.
export GDK_DEBUG=no-portals GTK_USE_PORTAL=0
log=$(mktemp)
# Every capture starts from a clean application state.
state=$(mktemp -d)
export PAM_DESKTOP_STATE_DIR=$state PAM_DESKTOP_DATA_DIR=$state/data PAM_DESKTOP_CACHE_DIR=$state/cache
PAM_DESKTOP_WINDOW=$size timeout $(( seconds + 30 )) "${PAM_DESKTOP_HOST:-$root/target/debug/pam-desktop}" "${PAM_SHOT_MODE:-run}" "$app" >"$log" 2>&1 &
host=$!
sleep "$seconds"
if ! kill -0 "$host" 2>/dev/null; then
    cat "$log" >&2
    wait "$host" 2>/dev/null || true
    rm -rf "$log" "$state"
    exit 1
fi
# The xdotool arguments may hold several commands separated by a literal
# "then" (needed after `type`, which consumes every following argument).
step=()
for argument in "$@" then; do
    if [ "$argument" = then ]; then
        [ "${#step[@]}" -gt 0 ] && xdotool "${step[@]}"
        step=()
    else
        step+=("$argument")
    fi
done
[ "$#" -gt 0 ] && sleep 1
import -window root -crop "${size}+0+0" +repage "$output"
kill "$host" 2>/dev/null || true
wait "$host" 2>/dev/null || true
grep -v 'DRI3\|MESA\|Note: you can' "$log" | head -40 || true
rm -rf "$log" "$state"

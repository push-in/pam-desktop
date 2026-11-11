#!/usr/bin/env bash
# Runs the end-to-end scripts of the examples (examples/<name>/tests/e2e.json)
# against the real host: every step is a user action or a check on the live
# window (see PAM_DESKTOP_SCRIPT in docs/development.md).
#
#   scripts/e2e.sh [example…]     default: every example with a script
#
# The examples run from PAM_E2E_DIR when scripts/e2e-setup.sh prepared them
# there (against this working tree), else from examples/ as installed.
#
# Linux uses a private virtual display when none is set (Xvfb), macOS and
# Windows their desktop. PAM_DESKTOP_HOST picks the host binary.
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
apps=${PAM_E2E_DIR:-${RUNNER_TEMP:-/tmp}/pam-e2e}
[ -d "$apps" ] || apps=$root/examples
host=${PAM_DESKTOP_HOST:-$root/target/debug/pam-desktop}
[ -x "$host" ] || host="$host.exe"
examples=("$@")
if [ ${#examples[@]} -eq 0 ]; then
    for script in "$apps"/*/tests/e2e.json; do
        examples+=("$(basename "$(dirname "$(dirname "$script")")")")
    done
fi
if [ "$(uname -s)" = Linux ] && [ -z "${DISPLAY:-}" ]; then
    display=:${PAM_E2E_DISPLAY:-93}
    Xvfb "$display" -screen 0 1600x1200x24 -nolisten tcp >/dev/null 2>&1 &
    xvfb=$!
    trap 'kill $xvfb 2>/dev/null' EXIT
    export DISPLAY=$display
    sleep 1
fi
# Paths for the host: Windows wants its own form (Git Bash gives /c/…).
native() {
    if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
# `timeout` is not on macOS: the run is killed after $1 seconds.
limited() {
    local seconds=$1
    shift
    "$@" &
    local pid=$!
    ( sleep "$seconds" && kill "$pid" 2>/dev/null ) &
    local watchdog=$!
    disown "$watchdog" 2>/dev/null
    wait "$pid"
    local status=$?
    kill "$watchdog" 2>/dev/null
    return $status
}
failed=0
for example in "${examples[@]}"; do
    state=$(mktemp -d)
    log=$(mktemp)
    started=$(date +%s)
    PAM_DESKTOP_SCRIPT=$(native "$apps/$example/tests/e2e.json") \
        PAM_DESKTOP_STATE_DIR=$(native "$state") PAM_DESKTOP_DATA_DIR=$(native "$state/data") PAM_DESKTOP_CACHE_DIR=$(native "$state/cache") \
        PAM_DESKTOP_WINDOW=1400x860 GDK_DEBUG=no-portals GTK_USE_PORTAL=0 \
        limited 180 "$host" run "$(native "$apps/$example")" >"$log" 2>&1
    status=$?
    seconds=$(( $(date +%s) - started ))
    if [ $status -eq 0 ] && grep -q "pam-desktop script: passed" "$log"; then
        echo "e2e $example: passed (${seconds}s)"
    else
        echo "e2e $example: FAILED (exit $status, ${seconds}s)"
        grep -v 'DRI3\|MESA\|no-portals' "$log" | tail -20
        failed=1
    fi
    rm -rf "$state" "$log"
done
exit $failed

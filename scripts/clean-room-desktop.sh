#!/bin/sh

set -eu

fail()
{
    printf 'PAM Desktop clean-room: %s\n' "$1" >&2
    exit 1
}

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PAM_CLI=${PAM_CLEAN_ROOM_PAM:-pam}
HOST=${PAM_DESKTOP_HOST_BINARY:-"$ROOT/target/release/pam-desktop"}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/pam-desktop-clean-room.XXXXXX")
APP="$WORK/workstation"

cleanup()
{
    if [ -d "$APP" ]; then
        (cd "$APP" && "$PAM_CLI" desktop clean >/dev/null 2>&1) || true
    fi
    rm -rf -- "$WORK"
    (cd "$ROOT" && cargo clean >/dev/null 2>&1) || true
}
trap cleanup EXIT HUP INT TERM

command -v "$PAM_CLI" >/dev/null 2>&1 || fail 'pam CLI is unavailable'
command -v composer >/dev/null 2>&1 || fail 'Composer is unavailable'
[ -x "$HOST" ] || fail "candidate host is not executable: $HOST"

"$PAM_CLI" init "$APP" --template desktop --no-interaction
cd "$APP"

# The generated app is rebound to this checkout, so the gate cannot pass by
# accidentally exercising the previously published Composer package.
"$PAM_CLI" composer config repositories.pam-desktop \
    "{\"type\":\"path\",\"url\":\"$ROOT/packages/desktop\",\"options\":{\"symlink\":false,\"versions\":{\"pushinbr/pam-desktop\":\"1.2.99\"}}}" \
    --json
"$PAM_CLI" composer update pushinbr/pam-desktop --with-dependencies --no-interaction --prefer-dist

export PAM_DESKTOP_HOST_BINARY="$HOST"
"$PAM_CLI" desktop doctor

# Dev is bounded: readiness must be reached, then the host is terminated.
DEV_LOG="$WORK/dev.log"
"$PAM_CLI" desktop dev >"$DEV_LOG" 2>&1 &
DEV_PID=$!
ready=0
attempt=0
while [ "$attempt" -lt 120 ]; do
    attempt=$((attempt + 1))
    if grep -Eq 'Gateway|ready|listening|http://127\.0\.0\.1' "$DEV_LOG"; then
        ready=1
        break
    fi
    kill -0 "$DEV_PID" 2>/dev/null || break
    sleep 1
done
kill "$DEV_PID" 2>/dev/null || true
wait "$DEV_PID" 2>/dev/null || true
[ "$ready" -eq 1 ] || { sed -n '1,200p' "$DEV_LOG" >&2; fail 'pam desktop dev did not become ready'; }

"$PAM_CLI" desktop build --format directory --force
PACKAGE=$(find dist -mindepth 1 -maxdepth 1 -type d -print -quit)
[ -n "$PACKAGE" ] || fail 'directory package was not created'
MANIFEST="$PACKAGE/manifest.json"
[ -f "$MANIFEST" ] || fail 'package integrity manifest is missing'

# Installation is exercised through the generated portable installer. It is
# redirected into the disposable home and then launched with --version.
INSTALLER=$(find "$PACKAGE" -type f -name install.sh -print -quit)
[ -n "$INSTALLER" ] || fail 'portable installer is missing'
FAKE_HOME="$WORK/home"
mkdir -p "$FAKE_HOME"
HOME="$FAKE_HOME" XDG_DATA_HOME="$FAKE_HOME/.local/share" XDG_BIN_HOME="$FAKE_HOME/.local/bin" sh "$INSTALLER"
INSTALLED=$(find "$FAKE_HOME/.local/bin" -mindepth 1 -maxdepth 1 -type l -print | head -n 1)
[ -n "$INSTALLED" ] || fail 'installed application executable was not found'
[ -x "$INSTALLED" ] || fail 'installed application link does not resolve to an executable'
"$INSTALLED" --version >/dev/null

printf '[ok] PAM Desktop clean-room create/dev/package/install/launch passed.\n'

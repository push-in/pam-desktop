#!/bin/sh

set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
FIXTURE=$(mktemp -d "${TMPDIR:-/tmp}/pam-desktop-cache.XXXXXX")
trap 'rm -rf -- "$FIXTURE"' EXIT HUP INT TERM

for version in 0.9.0 1.0.0 1.1.0 1.2.1 1.2.5 1.2.7
do
    directory="${FIXTURE}/${version}/x86_64-unknown-linux-gnu/bin"
    mkdir -p "$directory"
    printf '#!/bin/sh\nprintf "host %%s\\n" "$*"\n' >"${directory}/pam-desktop"
    chmod +x "${directory}/pam-desktop"
    sleep 1
done

mkdir -p "${FIXTURE}/.download-1.2.0.stale"
touch -d '3 days ago' "${FIXTURE}/.download-1.2.0.stale"

output=$(PAM_DESKTOP_CACHE_DIR="$FIXTURE" \
    "$ROOT/packages/desktop/bin/pam-desktop" doctor)

[ "$output" = 'host doctor' ]
[ ! -e "${FIXTURE}/0.9.0" ]
[ ! -e "${FIXTURE}/1.0.0" ]
[ ! -e "${FIXTURE}/1.1.0" ]
[ -d "${FIXTURE}/1.2.1" ]
[ -d "${FIXTURE}/1.2.5" ]
[ -d "${FIXTURE}/1.2.7" ]
[ ! -e "${FIXTURE}/.download-1.2.0.stale" ]

printf 'PAM Desktop host cache retention passed.\n'

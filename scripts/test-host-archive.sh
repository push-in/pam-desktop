#!/bin/sh

set -eu

usage()
{
    printf 'Usage: %s <pam-desktop-host.tar.gz>\n' "$0" >&2
    exit 2
}

[ "$#" -eq 1 ] || usage

ARCHIVE=$1
CHECKSUM="${ARCHIVE}.sha256"
[ -f "$ARCHIVE" ] || { printf 'Archive not found: %s\n' "$ARCHIVE" >&2; exit 1; }
[ -f "$CHECKSUM" ] || { printf 'Checksum not found: %s\n' "$CHECKSUM" >&2; exit 1; }
command -v jq >/dev/null 2>&1 \
    || { printf 'jq is required to validate the host manifest.\n' >&2; exit 1; }

ARCHIVE=$(CDPATH= cd -- "$(dirname -- "$ARCHIVE")" && pwd -P)/$(basename "$ARCHIVE")
CHECKSUM="${ARCHIVE}.sha256"

(
    cd "$(dirname -- "$ARCHIVE")"
    sha256sum --check "$(basename "$CHECKSUM")"
)

WORK_DIRECTORY=$(mktemp -d)
cleanup()
{
    rm -rf "$WORK_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

tar -tzf "$ARCHIVE" | while IFS= read -r member
do
    case "$member" in
        /*|../*|*/../*|*/..) printf 'Unsafe archive member: %s\n' "$member" >&2; exit 1 ;;
    esac
done
tar -tvzf "$ARCHIVE" | while IFS= read -r member
do
    case "$member" in
        d*|-*) ;;
        *) printf 'Archive contains a non-file member: %s\n' "$member" >&2; exit 1 ;;
    esac
done
tar -xzf "$ARCHIVE" -C "$WORK_DIRECTORY"

TOP_LEVEL_COUNT=$(find "$WORK_DIRECTORY" -mindepth 1 -maxdepth 1 -printf '.\n' | wc -l)
[ "$TOP_LEVEL_COUNT" -eq 1 ] \
    || { printf 'Archive must contain exactly one top-level entry.\n' >&2; exit 1; }
set -- "$WORK_DIRECTORY"/pam-desktop-*
[ "$#" -eq 1 ] && [ -d "$1" ] && [ ! -L "$1" ] \
    || { printf 'Archive must contain one PAM Desktop root directory.\n' >&2; exit 1; }
PACKAGE_DIRECTORY=$1
MANIFEST="${PACKAGE_DIRECTORY}/manifest.json"

jq -e '
    .schemaVersion == 1
    and .apiVersion == 1
    and .protocolVersion == 6
    and .target == "x86_64-unknown-linux-gnu"
    and (.version | test("^[0-9]+\\.[0-9]+\\.[0-9]+"))
    and (.files | type == "array" and length == 6)
' "$MANIFEST" >/dev/null

EXPECTED_FILES='LICENSE
README.md
bin/pam-desktop
bin/pam-desktop-launcher
install.sh
manifest.json
uninstall.sh'
ACTUAL_FILES=$(find "$PACKAGE_DIRECTORY" -type f -printf '%P\n' | sort)
[ "$ACTUAL_FILES" = "$EXPECTED_FILES" ] \
    || { printf 'Host archive contains an unexpected file set.\n%s\n' "$ACTUAL_FILES" >&2; exit 1; }
[ -x "${PACKAGE_DIRECTORY}/bin/pam-desktop" ]
[ -x "${PACKAGE_DIRECTORY}/bin/pam-desktop-launcher" ]
[ -x "${PACKAGE_DIRECTORY}/install.sh" ]
[ -x "${PACKAGE_DIRECTORY}/uninstall.sh" ]

jq -r '.files[] | [.path, (.bytes | tostring), .sha256] | @tsv' "$MANIFEST" \
    | while IFS='	' read -r relative expected_bytes expected_digest
do
    case "$relative" in
        ''|/*|../*|*/../*|*/..) printf 'Unsafe manifest path: %s\n' "$relative" >&2; exit 1 ;;
    esac
    file="${PACKAGE_DIRECTORY}/${relative}"
    [ -f "$file" ] && [ ! -L "$file" ] \
        || { printf 'Manifest file is missing or unsafe: %s\n' "$relative" >&2; exit 1; }
    actual_bytes=$(wc -c < "$file" | tr -d ' ')
    actual_digest=$(sha256sum "$file" | cut -d ' ' -f 1)
    [ "$actual_bytes" = "$expected_bytes" ] \
        || { printf 'Byte length mismatch: %s\n' "$relative" >&2; exit 1; }
    [ "$actual_digest" = "$expected_digest" ] \
        || { printf 'SHA-256 mismatch: %s\n' "$relative" >&2; exit 1; }
done

VERSION=$(jq -r '.version' "$MANIFEST")
[ "$("${PACKAGE_DIRECTORY}/bin/pam-desktop" --version)" = "pam-desktop ${VERSION}" ]

XDG_DATA_HOME="${WORK_DIRECTORY}/xdg-data"
XDG_BIN_HOME="${WORK_DIRECTORY}/xdg-bin"
export XDG_DATA_HOME XDG_BIN_HOME
"${PACKAGE_DIRECTORY}/install.sh"
[ "$("${XDG_BIN_HOME}/pam-desktop" --version)" = "pam-desktop ${VERSION}" ]
[ "$(readlink "${XDG_BIN_HOME}/pam-desktop")" = \
    "${XDG_DATA_HOME}/pam-desktop/${VERSION}/bin/pam-desktop" ]
"${XDG_DATA_HOME}/pam-desktop/${VERSION}/uninstall.sh"
[ ! -e "${XDG_DATA_HOME}/pam-desktop/${VERSION}" ]
[ ! -e "${XDG_BIN_HOME}/pam-desktop" ]
[ ! -e "${XDG_BIN_HOME}/pam-desktop-launcher" ]

trap - EXIT HUP INT TERM
rm -rf "$WORK_DIRECTORY"
printf '[ok] Linux host archive contract passed: %s\n' "$ARCHIVE"

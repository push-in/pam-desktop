#!/bin/sh

set -eu

usage()
{
    printf 'Usage: %s <binary-directory> <output-directory> <version>\n' "$0" >&2
    exit 2
}

[ "$#" -eq 3 ] || usage

BINARY_DIR=$1
OUTPUT_DIR=$2
VERSION=${3#v}
TARGET='x86_64-unknown-linux-gnu'
API_VERSION=1
PROTOCOL_VERSION=6
SOURCE_EPOCH=${SOURCE_DATE_EPOCH:-0}
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
REPOSITORY_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd -P)

printf '%s\n' "$VERSION" \
    | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$' \
    || usage
case "$SOURCE_EPOCH" in
    ''|*[!0-9]*) usage ;;
esac

[ "$(uname -s)" = 'Linux' ] \
    || { printf 'PAM Desktop host packages can only be built on Linux.\n' >&2; exit 1; }
case "$(uname -m)" in
    x86_64|amd64) ;;
    *)
        printf 'Expected an x86-64 build host.\n' >&2
        exit 1
        ;;
esac

for executable in pam-desktop pam-desktop-launcher
do
    [ -f "${BINARY_DIR}/${executable}" ] \
        || { printf 'Missing %s/%s.\n' "$BINARY_DIR" "$executable" >&2; exit 1; }
    [ -x "${BINARY_DIR}/${executable}" ] \
        || { printf '%s/%s is not executable.\n' "$BINARY_DIR" "$executable" >&2; exit 1; }
    [ ! -L "${BINARY_DIR}/${executable}" ] \
        || { printf '%s/%s must not be a symbolic link.\n' "$BINARY_DIR" "$executable" >&2; exit 1; }
done

HOST_VERSION=$("${BINARY_DIR}/pam-desktop" --version)
[ "$HOST_VERSION" = "pam-desktop ${VERSION}" ] || {
    printf 'Host version mismatch: expected %s, received %s.\n' \
        "pam-desktop ${VERSION}" "$HOST_VERSION" >&2
    exit 1
}

PACKAGE_NAME="pam-desktop-${VERSION}-${TARGET}"
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd -P)
ARCHIVE="${OUTPUT_DIR}/${PACKAGE_NAME}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"
[ ! -e "$ARCHIVE" ] && [ ! -e "$CHECKSUM" ] || {
    printf 'Refusing to replace an existing host artifact: %s\n' "$ARCHIVE" >&2
    exit 1
}

WORK_DIRECTORY=$(mktemp -d)
PACKAGE_DIRECTORY="${WORK_DIRECTORY}/${PACKAGE_NAME}"
cleanup()
{
    rm -rf "$WORK_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "${PACKAGE_DIRECTORY}/bin"
install -m 0755 \
    "${BINARY_DIR}/pam-desktop" \
    "${PACKAGE_DIRECTORY}/bin/pam-desktop"
install -m 0755 \
    "${BINARY_DIR}/pam-desktop-launcher" \
    "${PACKAGE_DIRECTORY}/bin/pam-desktop-launcher"
install -m 0644 \
    "${REPOSITORY_ROOT}/packaging/linux/README.md" \
    "${PACKAGE_DIRECTORY}/README.md"
install -m 0644 "${REPOSITORY_ROOT}/LICENSE" "${PACKAGE_DIRECTORY}/LICENSE"
sed \
    -e "s/@VERSION@/${VERSION}/g" \
    -e "s/@TARGET@/${TARGET}/g" \
    "${REPOSITORY_ROOT}/packaging/linux/install.sh" \
    > "${PACKAGE_DIRECTORY}/install.sh"
sed \
    -e "s/@VERSION@/${VERSION}/g" \
    "${REPOSITORY_ROOT}/packaging/linux/uninstall.sh" \
    > "${PACKAGE_DIRECTORY}/uninstall.sh"
chmod 0755 "${PACKAGE_DIRECTORY}/install.sh" "${PACKAGE_DIRECTORY}/uninstall.sh"

FILES='LICENSE
README.md
bin/pam-desktop
bin/pam-desktop-launcher
install.sh
uninstall.sh'

{
    printf '{\n'
    printf '  "schemaVersion": 1,\n'
    printf '  "apiVersion": %s,\n' "$API_VERSION"
    printf '  "protocolVersion": %s,\n' "$PROTOCOL_VERSION"
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "target": "%s",\n' "$TARGET"
    printf '  "files": [\n'
    first=1
    printf '%s\n' "$FILES" | while IFS= read -r relative
    do
        file="${PACKAGE_DIRECTORY}/${relative}"
        bytes=$(wc -c < "$file" | tr -d ' ')
        digest=$(sha256sum "$file" | cut -d ' ' -f 1)
        if [ "$first" -eq 0 ]; then
            printf ',\n'
        fi
        first=0
        printf '    {"path": "%s", "bytes": %s, "sha256": "%s"}' \
            "$relative" "$bytes" "$digest"
    done
    printf '\n  ]\n'
    printf '}\n'
} > "${PACKAGE_DIRECTORY}/manifest.json"

tar \
    --format=gnu \
    --sort=name \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --mtime="@${SOURCE_EPOCH}" \
    -C "$WORK_DIRECTORY" \
    -cf - \
    "$PACKAGE_NAME" \
    | gzip -n -9 > "$ARCHIVE"

archive_digest=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
printf '%s  %s\n' "$archive_digest" "$(basename "$ARCHIVE")" > "$CHECKSUM"

trap - EXIT HUP INT TERM
rm -rf "$WORK_DIRECTORY"
printf '[ok] Linux host archive: %s\n' "$ARCHIVE"
printf '[ok] SHA-256: %s\n' "$CHECKSUM"

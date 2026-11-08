#!/bin/sh
# Packs a built host as the archive the Composer launcher downloads:
#
#   scripts/package-host.sh <host binary> <output directory> <version> <target>
#
# writes pam-desktop-<version>-<target>.tar.gz and its .sha256. The archive
# holds pam-desktop-<version>-<target>/bin/pam-desktop[.exe] next to the
# license files, with fixed owners and dates (SOURCE_DATE_EPOCH, default 0),
# so the same binary always gives the same archive.

set -eu

usage()
{
    printf 'Usage: %s <host binary> <output directory> <version> <target>\n' "$0" >&2
    exit 2
}

[ "$#" -eq 4 ] || usage

BINARY=$1
OUTPUT_DIR=$2
VERSION=${3#v}
TARGET=$4
SOURCE_EPOCH=${SOURCE_DATE_EPOCH:-0}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)

printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$' || usage
case "$SOURCE_EPOCH" in '' | *[!0-9]*) usage ;; esac
case "$TARGET" in
    x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu | x86_64-apple-darwin | aarch64-apple-darwin) NAME=pam-desktop ;;
    x86_64-pc-windows-msvc | aarch64-pc-windows-msvc) NAME=pam-desktop.exe ;;
    *) printf 'Unknown target %s.\n' "$TARGET" >&2; exit 2 ;;
esac

[ -f "$BINARY" ] && [ ! -L "$BINARY" ] || { printf '%s is not a regular file.\n' "$BINARY" >&2; exit 1; }

# The binary must be the version it is published as, when it can run here.
if REPORTED=$("$BINARY" --version 2>/dev/null); then
    [ "$REPORTED" = "pam-desktop ${VERSION}" ] || {
        printf 'Host version mismatch: expected "pam-desktop %s", the binary reports "%s".\n' "$VERSION" "$REPORTED" >&2
        exit 1
    }
fi

PACKAGE="pam-desktop-${VERSION}-${TARGET}"
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd -P)
ARCHIVE="${OUTPUT_DIR}/${PACKAGE}.tar.gz"
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/pam-desktop-host.XXXXXX")
trap 'rm -rf -- "$STAGE"' EXIT HUP INT TERM

mkdir -p "${STAGE}/${PACKAGE}/bin"
cp "$BINARY" "${STAGE}/${PACKAGE}/bin/${NAME}"
chmod 755 "${STAGE}/${PACKAGE}/bin/${NAME}"
for document in LICENSE NOTICE; do
    [ ! -f "${ROOT}/${document}" ] || cp "${ROOT}/${document}" "${STAGE}/${PACKAGE}/${document}"
done

# The same modes whatever the umask of the build machine.
chmod -R u=rwX,go=rX "${STAGE}/${PACKAGE}"

# GNU tar pins order, owners and dates; bsdtar (macOS) has its own spelling.
if tar --version 2>/dev/null | grep -q 'GNU tar'; then
    tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${SOURCE_EPOCH}" -C "$STAGE" -cf "${STAGE}/host.tar" "$PACKAGE"
else
    find "${STAGE}/${PACKAGE}" -exec touch -t "$(date -u -r "$SOURCE_EPOCH" +%Y%m%d%H%M.%S)" {} +
    (cd "$STAGE" && find "$PACKAGE" -print | LC_ALL=C sort | tar --uid 0 --gid 0 --no-recursion -cf "${STAGE}/host.tar" -T -)
fi
gzip -n -9 <"${STAGE}/host.tar" >"$ARCHIVE"

if command -v sha256sum >/dev/null 2>&1; then
    DIGEST=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
else
    DIGEST=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
fi
printf '%s  %s\n' "$DIGEST" "${PACKAGE}.tar.gz" >"${ARCHIVE}.sha256"
printf '%s\n' "$ARCHIVE"

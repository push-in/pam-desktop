#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
WORK_DIRECTORY=$(mktemp -d)
cleanup()
{
    rm -rf "$WORK_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

PACKAGE_NAME='pam-desktop-1.2.1-x86_64-unknown-linux-gnu'
PACKAGE="${WORK_DIRECTORY}/${PACKAGE_NAME}"
mkdir -p "${PACKAGE}/bin"
printf 'host-binary' > "${PACKAGE}/bin/pam-desktop"
printf 'launcher' > "${PACKAGE}/bin/pam-desktop-launcher"
printf 'license' > "${PACKAGE}/LICENSE"
printf 'readme' > "${PACKAGE}/README.md"
printf 'install' > "${PACKAGE}/install.sh"
printf 'uninstall' > "${PACKAGE}/uninstall.sh"
license_digest=$(sha256sum "${PACKAGE}/LICENSE" | cut -d ' ' -f 1)
readme_digest=$(sha256sum "${PACKAGE}/README.md" | cut -d ' ' -f 1)
host_digest=$(sha256sum "${PACKAGE}/bin/pam-desktop" | cut -d ' ' -f 1)
launcher_digest=$(sha256sum "${PACKAGE}/bin/pam-desktop-launcher" | cut -d ' ' -f 1)
install_digest=$(sha256sum "${PACKAGE}/install.sh" | cut -d ' ' -f 1)
uninstall_digest=$(sha256sum "${PACKAGE}/uninstall.sh" | cut -d ' ' -f 1)
jq -n '
    {
        schemaVersion: 1,
        apiVersion: 1,
        protocolVersion: 6,
        version: "1.2.1",
        target: "x86_64-unknown-linux-gnu",
        files: [
            {path: "LICENSE", bytes: 7, sha256: $license_digest},
            {path: "README.md", bytes: 6, sha256: $readme_digest},
            {path: "bin/pam-desktop", bytes: 11, sha256: $host_digest},
            {path: "bin/pam-desktop-launcher", bytes: 8, sha256: $launcher_digest},
            {path: "install.sh", bytes: 7, sha256: $install_digest},
            {path: "uninstall.sh", bytes: 9, sha256: $uninstall_digest}
        ]
    }
' \
    --arg license_digest "$license_digest" \
    --arg readme_digest "$readme_digest" \
    --arg host_digest "$host_digest" \
    --arg launcher_digest "$launcher_digest" \
    --arg install_digest "$install_digest" \
    --arg uninstall_digest "$uninstall_digest" \
    > "${PACKAGE}/manifest.json"
ARCHIVE="${WORK_DIRECTORY}/${PACKAGE_NAME}.tar.gz"
tar -C "$WORK_DIRECTORY" -czf "$ARCHIVE" "$PACKAGE_NAME"
EVIDENCE="${WORK_DIRECTORY}/footprint.json"
PAM_EVIDENCE_REVISION=0123456789abcdef0123456789abcdef01234567 \
    "${SCRIPT_DIR}/desktop-footprint-evidence.sh" create "$ARCHIVE" "$EVIDENCE"
"${SCRIPT_DIR}/desktop-footprint-evidence.sh" verify "$ARCHIVE" "$EVIDENCE"

jq -e '
    .schema_version == 1
    and .suite_id == 2
    and .surface_code == 3
    and .result_code == 1
    and .source_revision == "0123456789abcdef0123456789abcdef01234567"
    and .footprint.executable_bytes == 19
    and .footprint.installed_bytes > .footprint.executable_bytes
' "$EVIDENCE" >/dev/null

BASELINE="${WORK_DIRECTORY}/baseline.json"
cp "$EVIDENCE" "$BASELINE"
"${SCRIPT_DIR}/desktop-footprint-evidence.sh" compare "$EVIDENCE" "$BASELINE" 5

printf 'changed-host' > "${PACKAGE}/bin/pam-desktop"
CORRUPT_ARCHIVE="${WORK_DIRECTORY}/corrupt/${PACKAGE_NAME}.tar.gz"
mkdir -p "$(dirname -- "$CORRUPT_ARCHIVE")"
tar -C "$WORK_DIRECTORY" -czf "$CORRUPT_ARCHIVE" "$PACKAGE_NAME"
if PAM_EVIDENCE_REVISION=0123456789abcdef0123456789abcdef01234567 \
    "${SCRIPT_DIR}/desktop-footprint-evidence.sh" create \
    "$CORRUPT_ARCHIVE" "${WORK_DIRECTORY}/corrupt.json" >/dev/null 2>&1
then
    printf 'Package member mismatch unexpectedly passed.\n' >&2
    exit 1
fi

jq '.footprint.installed_bytes = (.footprint.installed_bytes * 2)' \
    "$EVIDENCE" > "${WORK_DIRECTORY}/regressed.json"
if "${SCRIPT_DIR}/desktop-footprint-evidence.sh" compare \
    "${WORK_DIRECTORY}/regressed.json" "$BASELINE" 5 >/dev/null 2>&1
then
    printf 'Regressed footprint unexpectedly passed.\n' >&2
    exit 1
fi

printf 'tampered' >> "$ARCHIVE"
if "${SCRIPT_DIR}/desktop-footprint-evidence.sh" verify \
    "$ARCHIVE" "$EVIDENCE" >/dev/null 2>&1
then
    printf 'Tampered archive unexpectedly passed.\n' >&2
    exit 1
fi

trap - EXIT HUP INT TERM
rm -rf "$WORK_DIRECTORY"
printf '[ok] Desktop footprint evidence contract passed.\n'

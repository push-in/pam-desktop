#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
WORK_DIRECTORY=$(mktemp -d)
cleanup()
{
    rm -rf "$WORK_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

FIRST="${WORK_DIRECTORY}/first"
SECOND="${WORK_DIRECTORY}/second"
mkdir -p "$FIRST" "$SECOND"
NAME='pam-desktop-1.2.1-x86_64-unknown-linux-gnu.tar.gz'
printf 'deterministic archive\n' > "${FIRST}/${NAME}"
cp "${FIRST}/${NAME}" "${SECOND}/${NAME}"
MANIFEST="${WORK_DIRECTORY}/evidence/evidence-manifest.json"
PAM_EVIDENCE_REVISION=0123456789abcdef0123456789abcdef01234567 \
SOURCE_DATE_EPOCH=123456789 \
    "${SCRIPT_DIR}/host-reproducibility-evidence.sh" create \
    "${FIRST}/${NAME}" "${SECOND}/${NAME}" "$MANIFEST"
"${SCRIPT_DIR}/host-reproducibility-evidence.sh" verify "${FIRST}/${NAME}" "$MANIFEST"

jq -e '
    .schema_version == 1
    and .suite_id == 1
    and .surface_code == 3
    and .result_code == 1
    and .build_count == 2
    and .source_date_epoch == 123456789
    and .source_revision == "0123456789abcdef0123456789abcdef01234567"
' "$MANIFEST" >/dev/null

printf 'tampered\n' >> "${SECOND}/${NAME}"
if PAM_EVIDENCE_REVISION=0123456789abcdef0123456789abcdef01234567 \
    "${SCRIPT_DIR}/host-reproducibility-evidence.sh" create \
    "${FIRST}/${NAME}" "${SECOND}/${NAME}" "${WORK_DIRECTORY}/tampered.json" \
    >/dev/null 2>&1
then
    printf 'Tampered second build unexpectedly passed.\n' >&2
    exit 1
fi

jq '.artifact.sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"' \
    "$MANIFEST" > "${WORK_DIRECTORY}/forged.json"
if "${SCRIPT_DIR}/host-reproducibility-evidence.sh" verify \
    "${FIRST}/${NAME}" "${WORK_DIRECTORY}/forged.json" >/dev/null 2>&1
then
    printf 'Forged evidence unexpectedly passed.\n' >&2
    exit 1
fi

trap - EXIT HUP INT TERM
rm -rf "$WORK_DIRECTORY"
printf '[ok] Host reproducibility evidence contract passed.\n'

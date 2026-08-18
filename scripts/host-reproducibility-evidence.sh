#!/bin/sh

set -eu

SCHEMA_VERSION=1
SUITE_ID=1
SURFACE_CODE=3
RESULT_CODE_MATCH=1
MAX_ARCHIVE_BYTES=1073741824
MAX_SOURCE_DATE_EPOCH=253402300799

usage()
{
    printf 'Usage: %s create <first-archive> <second-archive> <manifest.json>\n' "$0" >&2
    printf '       %s verify <archive> <manifest.json>\n' "$0" >&2
    exit 64
}

fail()
{
    printf 'PAM Desktop reproducibility evidence: %s\n' "$1" >&2
    exit 1
}

require_tools()
{
    for tool in jq sha256sum cmp git stat uname wc
    do
        command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
    done
}

resolve_file()
{
    candidate=$1
    [ -f "$candidate" ] && [ ! -L "$candidate" ] \
        || fail "expected a regular non-symlink file: $candidate"
    directory=$(CDPATH= cd -- "$(dirname -- "$candidate")" && pwd -P)
    printf '%s/%s\n' "$directory" "$(basename -- "$candidate")"
}

archive_identity()
{
    archive_name=$(basename -- "$1")
    printf '%s\n' "$archive_name" \
        | grep -Eq '^pam-desktop-[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?-x86_64-unknown-linux-gnu\.tar\.gz$' \
        || fail "archive name does not match the stable Linux host contract: $archive_name"
}

archive_bytes()
{
    bytes=$(wc -c < "$1" | tr -d ' ')
    case "$bytes" in
        ''|*[!0-9]*) fail "cannot measure archive: $1" ;;
    esac
    [ "$bytes" -gt 0 ] && [ "$bytes" -le "$MAX_ARCHIVE_BYTES" ] \
        || fail "archive must contain 1 to $MAX_ARCHIVE_BYTES bytes: $1"
    printf '%s\n' "$bytes"
}

source_revision()
{
    revision=${PAM_EVIDENCE_REVISION:-}
    if [ -z "$revision" ]; then
        script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
        revision=$(git -C "${script_directory}/.." rev-parse --verify HEAD 2>/dev/null) \
            || fail 'PAM_EVIDENCE_REVISION is required outside a Git checkout'
    fi
    printf '%s\n' "$revision" | grep -Eq '^[0-9a-fA-F]{40}$' \
        || fail 'source revision must contain exactly 40 hexadecimal characters'
    printf '%s\n' "$revision" | tr 'A-F' 'a-f'
}

create_manifest()
{
    [ "$#" -eq 3 ] || usage
    first=$(resolve_file "$1")
    second=$(resolve_file "$2")
    output=$3
    [ "$first" != "$second" ] || fail 'two independent archive paths are required'
    [ "$(stat -c '%d:%i' "$first")" != "$(stat -c '%d:%i' "$second")" ] \
        || fail 'two independent archive files are required'
    archive_identity "$first"
    archive_identity "$second"
    [ "$(basename -- "$first")" = "$(basename -- "$second")" ] \
        || fail 'reproducibility builds must have the same archive name'
    first_bytes=$(archive_bytes "$first")
    second_bytes=$(archive_bytes "$second")
    [ "$first_bytes" = "$second_bytes" ] || fail 'archive byte lengths differ'
    cmp -s "$first" "$second" || fail 'independent host archives are not byte-identical'
    digest=$(sha256sum "$first" | cut -d ' ' -f 1)
    [ "$digest" = "$(sha256sum "$second" | cut -d ' ' -f 1)" ] \
        || fail 'independent host archive digests differ'
    epoch=${SOURCE_DATE_EPOCH:-0}
    case "$epoch" in
        0|[1-9]|[1-9][0-9]*) ;;
        *) fail 'SOURCE_DATE_EPOCH must be a canonical unsigned integer' ;;
    esac
    [ "$epoch" -le "$MAX_SOURCE_DATE_EPOCH" ] \
        || fail "SOURCE_DATE_EPOCH cannot exceed $MAX_SOURCE_DATE_EPOCH"
    revision=$(source_revision)
    output_parent=$(dirname -- "$output")
    mkdir -p "$output_parent"
    output_parent=$(CDPATH= cd -- "$output_parent" && pwd -P)
    output="${output_parent}/$(basename -- "$output")"
    [ ! -L "$output" ] && { [ ! -e "$output" ] || [ -f "$output" ]; } \
        || fail "manifest output must be a regular non-symlink path: $output"
    temporary="${output}.tmp.$$"
    trap 'rm -f "$temporary"' EXIT HUP INT TERM
    jq -n \
        --argjson schema_version "$SCHEMA_VERSION" \
        --argjson suite_id "$SUITE_ID" \
        --argjson surface_code "$SURFACE_CODE" \
        --argjson result_code "$RESULT_CODE_MATCH" \
        --argjson build_count 2 \
        --argjson source_date_epoch "$epoch" \
        --argjson bytes "$first_bytes" \
        --arg name "$(basename -- "$first")" \
        --arg sha256 "$digest" \
        --arg source_revision "$revision" \
        --arg host_os "$(uname -s)" \
        --arg host_arch "$(uname -m)" \
        '{
            schema_version: $schema_version,
            suite_id: $suite_id,
            surface_code: $surface_code,
            result_code: $result_code,
            build_count: $build_count,
            source_date_epoch: $source_date_epoch,
            source_revision: $source_revision,
            environment: {os: $host_os, architecture: $host_arch},
            artifact: {name: $name, bytes: $bytes, sha256: $sha256}
        }' > "$temporary"
    mv -f "$temporary" "$output"
    trap - EXIT HUP INT TERM
    printf '[ok] Reproducibility evidence: %s\n' "$output"
}

verify_manifest()
{
    [ "$#" -eq 2 ] || usage
    archive=$(resolve_file "$1")
    manifest=$(resolve_file "$2")
    archive_identity "$archive"
    jq -e \
        --argjson max_bytes "$MAX_ARCHIVE_BYTES" \
        --arg name "$(basename -- "$archive")" '
        type == "object"
        and .schema_version == 1
        and .suite_id == 1
        and .surface_code == 3
        and .result_code == 1
        and .build_count == 2
        and (.source_date_epoch | type == "number" and . >= 0 and floor == .)
        and (.source_revision | type == "string" and test("^[0-9a-f]{40}$"))
        and (.environment | keys | sort == ["architecture", "os"])
        and (.environment.os | type == "string" and length > 0 and length <= 128)
        and (.environment.architecture | type == "string" and length > 0 and length <= 128)
        and (.artifact | keys | sort == ["bytes", "name", "sha256"])
        and .artifact.name == $name
        and (.artifact.bytes | type == "number" and . > 0 and . <= $max_bytes and floor == .)
        and (.artifact.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
        and (keys | sort == ["artifact", "build_count", "environment", "result_code", "schema_version", "source_date_epoch", "source_revision", "suite_id", "surface_code"])
    ' "$manifest" >/dev/null || fail 'manifest schema or values are invalid'
    expected_bytes=$(jq -r '.artifact.bytes' "$manifest")
    expected_digest=$(jq -r '.artifact.sha256' "$manifest")
    [ "$(archive_bytes "$archive")" = "$expected_bytes" ] \
        || fail 'archive byte length does not match evidence'
    [ "$(sha256sum "$archive" | cut -d ' ' -f 1)" = "$expected_digest" ] \
        || fail 'archive digest does not match evidence'
    printf '[ok] Reproducibility evidence verified: %s\n' "$manifest"
}

require_tools
command=${1:-}
[ "$#" -gt 0 ] || usage
shift
case "$command" in
    create) create_manifest "$@" ;;
    verify) verify_manifest "$@" ;;
    *) usage ;;
esac

#!/bin/sh

set -eu

SCHEMA_VERSION=1
SUITE_ID=2
SURFACE_CODE=3
RESULT_CODE_PASS=1
MAX_ARCHIVE_BYTES=1073741824

usage()
{
    printf 'Usage: %s create <archive.tar.gz> <evidence.json>\n' "$0" >&2
    printf '       %s verify <archive.tar.gz> <evidence.json>\n' "$0" >&2
    printf '       %s compare <current.json> <baseline.json> [maximum-regression-percent]\n' "$0" >&2
    exit 64
}

fail()
{
    printf 'PAM Desktop footprint evidence: %s\n' "$1" >&2
    exit 1
}

require_tools()
{
    for tool in jq sha256sum git tar uname wc
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

package_manifest()
{
    archive=$1
    package_name=$(basename -- "$archive" .tar.gz)
    tar -xOzf "$archive" "${package_name}/manifest.json" 2>/dev/null \
        || fail 'archive does not contain its expected package manifest'
}

validate_package_manifest()
{
    jq -e '
        type == "object"
        and .schemaVersion == 1
        and .apiVersion == 1
        and (.protocolVersion | type == "number" and . >= 1 and floor == .)
        and .target == "x86_64-unknown-linux-gnu"
        and (.files | type == "array" and length == 6)
        and ([.files[].path] | sort == ["LICENSE", "README.md", "bin/pam-desktop", "bin/pam-desktop-launcher", "install.sh", "uninstall.sh"])
        and all(.files[];
            (.bytes | type == "number" and . >= 0 and floor == .)
            and (.sha256 | type == "string" and test("^[0-9a-f]{64}$")))
    ' >/dev/null || fail 'package manifest schema or values are invalid'
}

validate_package_files()
{
    archive=$1
    package_json=$2
    package_name=$(basename -- "$archive" .tar.gz)
    files='LICENSE
README.md
bin/pam-desktop
bin/pam-desktop-launcher
install.sh
uninstall.sh'
    printf '%s\n' "$files" | while IFS= read -r relative
    do
        expected_bytes=$(printf '%s\n' "$package_json" \
            | jq -r --arg path "$relative" '.files[] | select(.path == $path) | .bytes')
        expected_digest=$(printf '%s\n' "$package_json" \
            | jq -r --arg path "$relative" '.files[] | select(.path == $path) | .sha256')
        actual_bytes=$(tar -xOzf "$archive" "${package_name}/${relative}" 2>/dev/null | wc -c | tr -d ' ')
        actual_digest=$(tar -xOzf "$archive" "${package_name}/${relative}" 2>/dev/null | sha256sum | cut -d ' ' -f 1)
        [ "$actual_bytes" = "$expected_bytes" ] \
            || fail "package member byte length does not match manifest: $relative"
        [ "$actual_digest" = "$expected_digest" ] \
            || fail "package member digest does not match manifest: $relative"
    done
}

validate_evidence()
{
    jq -e \
        --argjson max_bytes "$MAX_ARCHIVE_BYTES" '
        type == "object"
        and .schema_version == 1
        and .suite_id == 2
        and .surface_code == 3
        and .result_code == 1
        and (.source_revision | type == "string" and test("^[0-9a-f]{40}$"))
        and (.environment | keys | sort == ["architecture", "os"])
        and (.environment.os | type == "string" and length > 0 and length <= 128)
        and (.environment.architecture | type == "string" and length > 0 and length <= 128)
        and (.artifact | keys | sort == ["bytes", "name", "sha256"])
        and (.artifact.name | type == "string" and test("^pam-desktop-[0-9]+\\.[0-9]+\\.[0-9]+([+-][0-9A-Za-z.-]+)?-x86_64-unknown-linux-gnu\\.tar\\.gz$"))
        and (.artifact.bytes | type == "number" and . > 0 and . <= $max_bytes and floor == .)
        and (.artifact.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
        and (.footprint | keys | sort == ["compression_ratio_ppm", "executable_bytes", "installed_bytes"])
        and (.footprint.installed_bytes | type == "number" and . > 0 and floor == .)
        and (.footprint.executable_bytes | type == "number" and . > 0 and floor == .)
        and (.footprint.executable_bytes <= .footprint.installed_bytes)
        and (.footprint.compression_ratio_ppm | type == "number" and . > 0 and floor == .)
        and (keys | sort == ["artifact", "environment", "footprint", "result_code", "schema_version", "source_revision", "suite_id", "surface_code"])
    ' "$1" >/dev/null || fail 'evidence schema or values are invalid'
}

measure_manifest()
{
    archive=$1
    package_json=$2
    package_name=$(basename -- "$archive" .tar.gz)
    installed_bytes=$(printf '%s\n' "$package_json" | jq '[.files[].bytes] | add')
    manifest_bytes=$(tar -xOzf "$archive" "${package_name}/manifest.json" 2>/dev/null \
        | wc -c | tr -d ' ')
    installed_bytes=$((installed_bytes + manifest_bytes))
    executable_bytes=$(printf '%s\n' "$package_json" \
        | jq '[.files[] | select(.path == "bin/pam-desktop" or .path == "bin/pam-desktop-launcher") | .bytes] | add')
}

create_evidence()
{
    [ "$#" -eq 2 ] || usage
    archive=$(resolve_file "$1")
    archive_identity "$archive"
    output=$2
    package_json=$(package_manifest "$archive")
    printf '%s\n' "$package_json" | validate_package_manifest
    validate_package_files "$archive" "$package_json"
    measure_manifest "$archive" "$package_json"
    measured_archive_bytes=$(archive_bytes "$archive")
    compression_ratio_ppm=$((measured_archive_bytes * 1000000 / installed_bytes))
    digest=$(sha256sum "$archive" | cut -d ' ' -f 1)
    revision=$(source_revision)
    output_parent=$(dirname -- "$output")
    mkdir -p "$output_parent"
    output_parent=$(CDPATH= cd -- "$output_parent" && pwd -P)
    output="${output_parent}/$(basename -- "$output")"
    [ ! -L "$output" ] && { [ ! -e "$output" ] || [ -f "$output" ]; } \
        || fail "evidence output must be a regular non-symlink path: $output"
    temporary="${output}.tmp.$$"
    trap 'rm -f "$temporary"' EXIT HUP INT TERM
    jq -n \
        --argjson schema_version "$SCHEMA_VERSION" \
        --argjson suite_id "$SUITE_ID" \
        --argjson surface_code "$SURFACE_CODE" \
        --argjson result_code "$RESULT_CODE_PASS" \
        --argjson archive_bytes "$measured_archive_bytes" \
        --argjson installed_bytes "$installed_bytes" \
        --argjson executable_bytes "$executable_bytes" \
        --argjson compression_ratio_ppm "$compression_ratio_ppm" \
        --arg name "$(basename -- "$archive")" \
        --arg sha256 "$digest" \
        --arg source_revision "$revision" \
        --arg host_os "$(uname -s)" \
        --arg host_arch "$(uname -m)" \
        '{
            schema_version: $schema_version,
            suite_id: $suite_id,
            surface_code: $surface_code,
            result_code: $result_code,
            source_revision: $source_revision,
            environment: {os: $host_os, architecture: $host_arch},
            artifact: {name: $name, bytes: $archive_bytes, sha256: $sha256},
            footprint: {
                installed_bytes: $installed_bytes,
                executable_bytes: $executable_bytes,
                compression_ratio_ppm: $compression_ratio_ppm
            }
        }' > "$temporary"
    mv -f "$temporary" "$output"
    trap - EXIT HUP INT TERM
    printf '[ok] Footprint evidence: %s\n' "$output"
}

verify_evidence()
{
    [ "$#" -eq 2 ] || usage
    archive=$(resolve_file "$1")
    evidence=$(resolve_file "$2")
    archive_identity "$archive"
    validate_evidence "$evidence"
    [ "$(jq -r '.artifact.name' "$evidence")" = "$(basename -- "$archive")" ] \
        || fail 'archive name does not match evidence'
    [ "$(jq -r '.artifact.bytes' "$evidence")" = "$(archive_bytes "$archive")" ] \
        || fail 'archive byte length does not match evidence'
    [ "$(jq -r '.artifact.sha256' "$evidence")" = "$(sha256sum "$archive" | cut -d ' ' -f 1)" ] \
        || fail 'archive digest does not match evidence'
    package_json=$(package_manifest "$archive")
    printf '%s\n' "$package_json" | validate_package_manifest
    validate_package_files "$archive" "$package_json"
    measure_manifest "$archive" "$package_json"
    [ "$(jq -r '.footprint.installed_bytes' "$evidence")" = "$installed_bytes" ] \
        || fail 'installed byte length does not match evidence'
    [ "$(jq -r '.footprint.executable_bytes' "$evidence")" = "$executable_bytes" ] \
        || fail 'executable byte length does not match evidence'
    expected_ratio=$(($(archive_bytes "$archive") * 1000000 / installed_bytes))
    [ "$(jq -r '.footprint.compression_ratio_ppm' "$evidence")" = "$expected_ratio" ] \
        || fail 'compression ratio does not match evidence'
    printf '[ok] Footprint evidence verified: %s\n' "$evidence"
}

compare_evidence()
{
    [ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
    current=$(resolve_file "$1")
    baseline=$(resolve_file "$2")
    maximum=${3:-5}
    case "$maximum" in
        0|[1-9]|[1-9][0-9]*) ;;
        *) fail 'maximum regression percent must be a canonical unsigned integer' ;;
    esac
    [ "$maximum" -le 100 ] || fail 'maximum regression percent cannot exceed 100'
    validate_evidence "$current"
    validate_evidence "$baseline"
    [ "$(jq -c '.environment' "$current")" = "$(jq -c '.environment' "$baseline")" ] \
        || fail 'current and baseline environments must match exactly'
    for path in artifact.bytes footprint.installed_bytes footprint.executable_bytes
    do
        current_value=$(jq -r ".$path" "$current")
        baseline_value=$(jq -r ".$path" "$baseline")
        allowed=$((baseline_value * (100 + maximum) / 100))
        [ "$current_value" -le "$allowed" ] \
            || fail "$path regressed from $baseline_value to $current_value bytes (maximum ${maximum}%)"
    done
    printf '[ok] Footprint regression gate passed (maximum %s%%).\n' "$maximum"
}

require_tools
command=${1:-}
[ "$#" -gt 0 ] || usage
shift
case "$command" in
    create) create_evidence "$@" ;;
    verify) verify_evidence "$@" ;;
    compare) compare_evidence "$@" ;;
    *) usage ;;
esac

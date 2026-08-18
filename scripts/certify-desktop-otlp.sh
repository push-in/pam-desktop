#!/usr/bin/env bash
set -euo pipefail

repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
collector_image='ghcr.io/open-telemetry/opentelemetry-collector-releases/opentelemetry-collector:0.157.0@sha256:4019ce4d7e7791a1a255fffb2f407af66d5017cc65543469ba565c4f47f795b8'
container="pam-desktop-otlp-${RANDOM}-$$"
target_dir=$(mktemp -d "${TMPDIR:-/tmp}/pam-desktop-otlp.XXXXXXXX")
image_existed=0

if docker image inspect "${collector_image}" >/dev/null 2>&1; then
    image_existed=1
fi

cleanup() {
    docker rm -f "${container}" >/dev/null 2>&1 || true
    cargo clean --target-dir "${target_dir}" >/dev/null 2>&1 || true
    if [[ "${target_dir}" == "${TMPDIR:-/tmp}/pam-desktop-otlp."* ]]; then
        rm -rf -- "${target_dir}"
    fi
    if [[ "${image_existed}" -eq 0 ]]; then
        docker image rm --force "${collector_image}" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

docker run --detach --name "${container}" \
    --read-only --cap-drop ALL --security-opt no-new-privileges \
    --publish 127.0.0.1::4318 \
    --volume "${repository}/tests/fixtures/otel-collector.yaml:/etc/otelcol/config.yaml:ro" \
    "${collector_image}" --config=/etc/otelcol/config.yaml >/dev/null

collector_port=$(docker port "${container}" 4318/tcp | awk -F: 'NR == 1 { print $NF }')
[[ "${collector_port}" =~ ^[0-9]+$ ]] || { printf 'Collector did not publish a valid port\n' >&2; exit 1; }

for _ in $(seq 1 60); do
    if curl --silent --output /dev/null --max-time 1 \
        "http://127.0.0.1:${collector_port}/v1/traces"; then
        break
    fi
    sleep 0.25
done

PAM_DESKTOP_OTLP_ENABLED=true \
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT="http://127.0.0.1:${collector_port}/v1/traces" \
OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/json \
OTEL_BSP_SCHEDULE_DELAY=25 \
CARGO_TARGET_DIR="${target_dir}" \
cargo test --locked -p pam-desktop --no-default-features --features gateway \
    desktop_otlp::tests::official_collector_accepts_desktop_command_span -- --ignored --exact

collector_log=$(docker logs "${container}" 2>&1)
grep -Fq 'pam.desktop.command' <<<"${collector_log}"
grep -Fq 'catalog.refresh' <<<"${collector_log}"
if grep -Fq 'must-not-leak' <<<"${collector_log}"; then
    printf 'Sensitive certification sentinel leaked into Collector output\n' >&2
    exit 1
fi
printf 'PAM Desktop OTLP Collector certification passed\n'

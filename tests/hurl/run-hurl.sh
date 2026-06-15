#!/usr/bin/env bash
# Boot the service against the checked-in SDE cache fixture (fully offline — no
# CCP or EVE-Scout calls) and run the HURL suites against it.
#
# The fixture mirrors the real on-disk cache layout (tests/fixtures/sde:
# latest.jsonl, metadata.json, <build>/*.jsonl), so the service loads it through
# the normal cache path. Reload + EVE-Scout pollers are disabled.
#
# Usage: tests/hurl/run-hurl.sh [PORT]
set -euo pipefail

PORT="${1:-5099}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASE_URL="http://localhost:${PORT}"

cargo build --quiet --bin erbridge-palu

PALU_CACHE_DIR="${ROOT}/tests/fixtures/sde" \
PALU_SDE_RELOAD_INTERVAL_SECS=0 \
PALU_EVE_SCOUT_INTERVAL_SECS=0 \
PALU_PORT="${PORT}" \
RUST_LOG="${RUST_LOG:-error}" \
    "${ROOT}/target/debug/erbridge-palu" &
SERVER_PID=$!
trap 'kill "${SERVER_PID}" 2>/dev/null || true' EXIT

# Wait for the server to come up.
for _ in $(seq 1 100); do
    if curl -sf "${BASE_URL}/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

hurl --test --variable "base_url=${BASE_URL}" "${ROOT}"/tests/hurl/*.hurl

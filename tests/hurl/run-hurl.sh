#!/usr/bin/env bash
# Boot the service against the checked-in SDE fixture (fully offline — no CCP or
# EVE-Scout calls), run the HURL suite against it, then tear it down.
#
# Usage: tests/hurl/run-hurl.sh [PORT]
set -euo pipefail

PORT="${1:-5099}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASE_URL="http://localhost:${PORT}"

cargo build --quiet --bin erbridge-geodesic

GEODESIC_SDE_DIR="${ROOT}/tests/fixtures/sde/1" \
GEODESIC_EVE_SCOUT_INTERVAL_SECS=0 \
GEODESIC_PORT="${PORT}" \
RUST_LOG="${RUST_LOG:-error}" \
    "${ROOT}/target/debug/erbridge-geodesic" &
SERVER_PID=$!
trap 'kill "${SERVER_PID}" 2>/dev/null || true' EXIT

# Wait for the server to come up.
for _ in $(seq 1 50); do
    if curl -sf "${BASE_URL}/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

hurl --test --variable "base_url=${BASE_URL}" "${ROOT}"/tests/hurl/*.hurl

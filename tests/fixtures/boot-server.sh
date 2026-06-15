#!/usr/bin/env bash
# Boot the service against the checked-in SDE cache fixture (fully offline — no
# CCP or EVE-Scout calls), wait for it to come up, run a command against it, then
# tear it down. Shared by the HURL runner and the load-test recipes so the
# boot/wait/teardown logic lives in one place.
#
# The fixture mirrors the real on-disk cache layout (tests/fixtures/sde:
# latest.jsonl, metadata.json, <build>/*.jsonl), so the service loads it through
# the normal cache path. Reload + EVE-Scout pollers are disabled.
#
# Usage: tests/fixtures/boot-server.sh PORT CMD [ARGS...]
#   PORT  port to bind the service on
#   CMD…  command run once the server is healthy; its exit status is propagated.
#         BASE_URL is exported into the command's environment.
#
# IMPORTANT: CMD is run in the FOREGROUND with the terminal attached and its
# stdout/stderr NOT piped — the load-test TUI must render live. Only the server
# is backgrounded.
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 PORT CMD [ARGS...]" >&2
    exit 64
fi

PORT="$1"
shift
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export BASE_URL="http://localhost:${PORT}"

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

# Run the requested command against the booted server (foreground, no pipe).
"$@"

#!/usr/bin/env bash
# Boot the service against the checked-in SDE cache fixture (fully offline — no
# CCP or EVE-Scout calls) and run the HURL suites against it.
#
# The boot/wait/teardown scaffolding lives in tests/fixtures/boot-server.sh
# (shared with the load-test recipes); this script just supplies the command to
# run once the server is healthy.
#
# Usage: tests/hurl/run-hurl.sh [PORT]
set -euo pipefail

PORT="${1:-5099}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# `--file-root "${ROOT}"` lets HURL `file,` bodies reference repo-root-relative
# paths (e.g. the committed load body under tests/load/), which HURL's default
# sandbox (the .hurl file's own dir) would otherwise refuse.
exec "${ROOT}/tests/fixtures/boot-server.sh" "${PORT}" \
    bash -c 'hurl --test --file-root "$1" --variable "base_url=${BASE_URL}" "${@:2}"' _ \
    "${ROOT}" "${ROOT}"/tests/hurl/*.hurl

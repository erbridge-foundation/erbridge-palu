# Image name matches the GHCR registry layout: ghcr.io/erbridge-foundation/palu
registry := "ghcr.io/erbridge-foundation"
image := registry + "/palu"

# ─── default: list available recipes ─────────────────────────────────────────
default:
    @just --list

# ─── full gate (CI parity) ───────────────────────────────────────────────────
# Everything CI runs: fmt, clippy, tests, and the HURL contract suite. Run this
# before pushing.
check: fmt clippy test hurl

# ─── lint / format ───────────────────────────────────────────────────────────

# Format check (does not modify files)
fmt:
    cargo fmt --all -- --check

# Apply formatting
fmt-fix:
    cargo fmt --all

# Lints, deny-warnings
clippy:
    cargo clippy --all-targets -- -D warnings

# ─── tests ───────────────────────────────────────────────────────────────────

# Unit + integration tests (offline)
test:
    cargo test --all-targets

# HURL HTTP-contract suite: boots the service on the SDE fixture (offline),
# runs tests/hurl/*.hurl, tears it down.
hurl port="5099":
    tests/hurl/run-hurl.sh {{port}}

# ─── load testing (local only — NEVER in CI) ─────────────────────────────────
# Drive `oha` against the fixture-booted server with a committed request body.
# Two bodies, two workloads:
#   • tests/load/fanout-wh.json  — one source, the 29-edge wormhole chain stated
#     once, 18 destinations (routes 1..=38 jumps): MANY routes per request, the
#     fan-out's intended workload.
#   • tests/load/single-route.json — a one-destination request (Jita→Amarr): ONE
#     route per request, comparable to the pre-fan-out single-route endpoint.
# NOT wired into `check`.
#
# req/s is not comparable across the two: the fan-out request does 18× the work,
# so its req/s is ~1/18th while its routes/sec is far higher. Compare routes/sec
# (req/s × destinations), not req/s, when weighing the two.
#
# CAVEAT (hot-graph upper bound): nothing here caches responses — every request
# re-runs the routing — but replaying one body keeps the CPU data cache and
# branch predictor warm over the same graph nodes, inflating throughput versus a
# real many-pilots workload. The diverse destination list mitigates this, so the
# numbers are a documented hot-graph UPPER BOUND, not a production estimate.
#
# `oha` is an opt-in dev tool, not a Cargo.toml dependency: `cargo install oha`.

# Internal: boot the fixture server and fire `oha` at it with the given body.
# `tui` is "" for the live TUI (foreground, no stdout pipe) or "--no-tui" for a
# capturable plain-text summary.
_load-test body tui port requests concurrency:
    #!/usr/bin/env sh
    set -e
    command -v oha >/dev/null 2>&1 || { echo "oha not found — install with: cargo install oha" >&2; exit 1; }
    # The body runs in the FOREGROUND with the terminal attached so oha's TUI
    # renders live; boot-server.sh backgrounds only the server.
    tests/fixtures/boot-server.sh {{port}} \
        oha {{tui}} -n {{requests}} -c {{concurrency}} -m POST \
            -H "content-type: application/json" \
            -D {{body}} \
            "http://localhost:{{port}}/api/v1/route/system"

# Fan-out load test (18 routes/request), oha live TUI
load-test port="5098" requests="2000" concurrency="50":
    @just _load-test tests/load/fanout-wh.json "" {{port}} {{requests}} {{concurrency}}

# Fan-out load test, --no-tui for a capturable plain-text summary
load-test-plain port="5098" requests="2000" concurrency="50":
    @just _load-test tests/load/fanout-wh.json --no-tui {{port}} {{requests}} {{concurrency}}

# Single-route load test (1 route/request, comparable to the old endpoint), live TUI
load-test-single port="5098" requests="20000" concurrency="100":
    @just _load-test tests/load/single-route.json "" {{port}} {{requests}} {{concurrency}}

# Single-route load test, --no-tui for a capturable plain-text summary
load-test-single-plain port="5098" requests="2000" concurrency="50":
    @just _load-test tests/load/single-route.json --no-tui {{port}} {{requests}} {{concurrency}}

# ─── fixtures ────────────────────────────────────────────────────────────────

# Regenerate the committed SDE test fixture from the latest CCP build (maps
# verbatim, type files trimmed to jump-capable hulls + the JDC skill). Whole
# fixture rebuilt at one build number so map and hull data never skew.
update-fixtures:
    tests/fixtures/update-sde-fixtures.sh

# ─── run ─────────────────────────────────────────────────────────────────────

# Run the service (downloads the live SDE on first start)
run:
    cargo run

# Run the service offline against the checked-in SDE fixture
run-fixture port="5001":
    #!/usr/bin/env sh
    set -e
    # Resolve the build-number subdir from the committed manifest so this keeps
    # working after a fixture refresh bumps the build number.
    build="$(sed -n 's/.*"buildNumber": *\([0-9]*\).*/\1/p' tests/fixtures/sde/latest.jsonl)"
    PALU_SDE_DIR="$PWD/tests/fixtures/sde/${build}" \
    PALU_EVE_SCOUT_INTERVAL_SECS=0 \
    PALU_PORT={{port}} \
        cargo run

# ─── docker ──────────────────────────────────────────────────────────────────

# Git-tag-derived version (leading "v" stripped; 0.0.0-dev.<sha> when no tag yet).
_app-version:
    #!/usr/bin/env sh
    sha="$(git rev-parse --short HEAD)"
    if git describe --tags --abbrev=0 >/dev/null 2>&1; then
        described="$(git describe --tags --always --dirty)"
        echo "${described#v}"
    else
        echo "0.0.0-dev.${sha}"
    fi

# Build the Docker image locally
docker-build:
    docker build -t {{image}}:latest .

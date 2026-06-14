# Image name matches the GHCR registry layout: ghcr.io/erbridge-foundation/geodesic
registry := "ghcr.io/erbridge-foundation"
image := registry + "/geodesic"

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
    GEODESIC_SDE_DIR="$PWD/tests/fixtures/sde/1" \
    GEODESIC_EVE_SCOUT_INTERVAL_SECS=0 \
    GEODESIC_PORT={{port}} \
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

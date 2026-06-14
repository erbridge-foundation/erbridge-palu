#!/usr/bin/env bash
# Regenerate the committed SDE test fixture from the latest CCP build.
#
# Rebuilds the WHOLE fixture at a single build number so the map graph and the
# hull catalog never skew across builds:
#   - map files (mapSolarSystems.jsonl, mapStargates.jsonl): extracted VERBATIM
#     (the full raw SDE — topology IS the routing test, so nothing is trimmed).
#   - type files (types.jsonl, typeDogma.jsonl): TRIMMED to just the rows the
#     hull catalog cares about. These two files are ~150 MB raw; committing them
#     whole would be bloat, and unlike the map files they have no topology to
#     preserve.
#
# TRIM PREDICATE — KEEP IN LOCKSTEP WITH THE PARSER (src/sde/parse.rs):
#   keep a `types.jsonl` row iff it is `published` AND its `typeDogma.jsonl` row
#   carries attribute 867 (jumpDriveRange); ALSO keep the JDC skill (typeID
#   21611) explicitly so attribute 870 (the per-level range bonus) survives.
# This is the same rule `parse_hull_catalog` applies, so the fixture is by
# construction exactly the rows the parser imports — the two cannot drift. If
# you change the parser's membership rule, change this predicate too.
#
# Outputs into tests/fixtures/sde/:
#   <build>/{mapSolarSystems,mapStargates,types,typeDogma}.jsonl
#   latest.jsonl   (verbatim CCP manifest)
#   metadata.json  (points at the new build)
# and prunes any previous build dir.
#
# Skips the (large) download entirely when the committed fixture is already at
# the latest build; pass --force to rebuild anyway.
#
# Requires: curl, unzip, jq.
# Usage: tests/fixtures/update-sde-fixtures.sh [--force]
set -euo pipefail

force=0
case "${1:-}" in
    --force | -f) force=1 ;;
    "") ;;
    *)
        echo "usage: $(basename "$0") [--force]" >&2
        exit 2
        ;;
esac

# Coupled to src/sde/types.rs — keep in sync.
ATTR_JUMP_DRIVE_RANGE=867
JDC_SKILL_TYPE_ID=21611

LATEST_URL="https://developers.eveonline.com/static-data/tranquility/latest.jsonl"
zip_url() {
    echo "https://developers.eveonline.com/static-data/tranquility/eve-online-static-data-${1}-jsonl.zip"
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FIXTURE_DIR="${ROOT}/tests/fixtures/sde"

for tool in curl unzip jq; do
    command -v "${tool}" >/dev/null 2>&1 || {
        echo "error: '${tool}' is required" >&2
        exit 1
    }
done

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

echo "fetching manifest: ${LATEST_URL}"
manifest="${work}/latest.jsonl"
curl -fsSL "${LATEST_URL}" -o "${manifest}"
build="$(jq -r '.buildNumber' < "${manifest}")"
echo "latest build: ${build}"

# Skip the download if the committed fixture already holds this build with all
# four files present. `--force` overrides (e.g. to recover a corrupt fixture).
cached_build=""
if [ -f "${FIXTURE_DIR}/metadata.json" ]; then
    cached_build="$(jq -r '.build_number // empty' < "${FIXTURE_DIR}/metadata.json")"
fi
fixture_complete=1
for f in mapSolarSystems.jsonl mapStargates.jsonl types.jsonl typeDogma.jsonl; do
    [ -f "${FIXTURE_DIR}/${build}/${f}" ] || fixture_complete=0
done
if [ "${force}" -eq 0 ] && [ "${cached_build}" = "${build}" ] && [ "${fixture_complete}" -eq 1 ]; then
    echo "fixture already at latest build ${build}; nothing to do (use --force to rebuild)"
    exit 0
fi

echo "downloading SDE zip for build ${build} (large)…"
zip="${work}/sde.zip"
curl -fSL "$(zip_url "${build}")" -o "${zip}"

echo "extracting map + type files…"
# Entries sit at the archive root; -j flattens, -o overwrites. Match exact
# names (no leading glob) so `types.jsonl` does not also pull `archetypes.jsonl`.
unzip -j -o "${zip}" \
    mapSolarSystems.jsonl mapStargates.jsonl \
    types.jsonl typeDogma.jsonl \
    -d "${work}/extract"

build_out="${FIXTURE_DIR}/${build}"
mkdir -p "${build_out}"

echo "copying map files verbatim…"
cp "${work}/extract/mapSolarSystems.jsonl" "${build_out}/mapSolarSystems.jsonl"
cp "${work}/extract/mapStargates.jsonl" "${build_out}/mapStargates.jsonl"

echo "trimming type files (published + attr ${ATTR_JUMP_DRIVE_RANGE}, plus JDC ${JDC_SKILL_TYPE_ID})…"
# Build the keep set as a JSON object {"<typeID>": true, …} once, then filter
# each file in a single jq pass (an O(1) key lookup per line — fast over the
# ~50k-line, 150 MB inputs; a per-line jq subprocess would take many minutes).
#
# 1. The keep set: typeIDs whose dogma carries attribute 867, plus the JDC skill
#    (kept explicitly so its attribute 870 — the per-level bonus — survives).
keep_set="${work}/keep_set.json"
{
    jq -r "select(any(.dogmaAttributes[]?; .attributeID == ${ATTR_JUMP_DRIVE_RANGE})) | ._key" \
        < "${work}/extract/typeDogma.jsonl"
    echo "${JDC_SKILL_TYPE_ID}"
} | jq -R 'tonumber' | jq -s 'INDEX(tostring)' > "${keep_set}"

# 2. types.jsonl: published rows whose typeID is in the keep set.
jq -c --slurpfile keep "${keep_set}" \
    'select(.published == true) | select($keep[0][(._key | tostring)])' \
    < "${work}/extract/types.jsonl" > "${build_out}/types.jsonl"

# 3. typeDogma.jsonl: the matching dogma rows (range-bearing types + JDC).
jq -c --slurpfile keep "${keep_set}" \
    'select($keep[0][(._key | tostring)])' \
    < "${work}/extract/typeDogma.jsonl" > "${build_out}/typeDogma.jsonl"

echo "writing manifest + metadata…"
cp "${manifest}" "${FIXTURE_DIR}/latest.jsonl"
release_date="$(jq -r '.releaseDate' < "${manifest}")"
jq -n \
    --argjson build "${build}" \
    --arg release "${release_date}" \
    '{build_number: $build, release_date: $release, fetched_at: $release}' \
    > "${FIXTURE_DIR}/metadata.json"

echo "pruning old build dirs…"
for d in "${FIXTURE_DIR}"/*/; do
    name="$(basename "${d}")"
    if [[ "${name}" =~ ^[0-9]+$ ]] && [[ "${name}" != "${build}" ]]; then
        echo "  removing ${name}"
        rm -rf "${d}"
    fi
done

echo "done. fixture rebuilt at build ${build}:"
wc -l "${build_out}"/*.jsonl

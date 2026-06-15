## Why

The blops staging endpoint answers a fast hunter question ("get my fleet into bridge
range of a fixed target"), but offers nothing for the inverse *planning* question a
pilot asks while sitting down to plan logistics or a roam: **"from this system, with
this hull and these skills, where can I jump?"** All the primitives to answer it —
the jump-range math, the kd-tree spatial index, and the hull catalog — already exist
and are exercised by blops; only the fan-out query and its response shape are missing.

This change also corrects a small validation bug the planning endpoint surfaces:
every jump-capable hull requires Jump Drive Calibration (JDC) level 1 at a minimum, so
a `jdc_level` of `0` is not a valid input for any jump query. The blops endpoint
currently accepts `0..=5`; this is wrong by the same universal mechanic and is fixed
here so both endpoints encode the rule once.

## What Changes

- Add `POST /api/v1/route/range`: a jump-range reachability fan-out. Given a source
  system, a required hull, and a required JDC level, it returns **every K-space system
  within one jump**, sorted nearest-first, plus a summary header.
- The new endpoint is **planning-oriented**, which drives three deliberate departures
  from the blops endpoint's fast-query conventions:
  - **Hull and `jdc_level` are required** — no worst-hull default, no defaulted level.
    A planner answer must be trustworthy, not silently assumed.
  - **An empty reachable set returns `200`** with `reachable: []`, not a `404`. "Your
    hull at this JDC reaches nothing in K-space" is a valid planning answer.
  - **No gate/wormhole overlay** — a jump ignores gates entirely, so avoid lists,
    wormhole connections, and Zarzakh opt-in do not apply.
- The reachable set excludes highsec destinations (a cyno cannot be lit in highsec),
  J-space (already excluded by the kd-tree), and the source system itself.
- **BREAKING (pre-prod)**: tighten the blops endpoint's `jdc_level` validation from
  `0..=5` to `1..=5`, rejecting `0`. Not shipped to production, so corrected now rather
  than left inconsistent.

## Capabilities

### New Capabilities
- `jump-range`: A jump-range reachability query — resolve a hull's effective range from
  the catalog at a required JDC level, fan out over the spatial index from a source
  system, and return the in-range, jump-legal K-space systems with a summary.

### Modified Capabilities
- `blops-staging`: The `jdc_level` validation range changes from `0..=5` to `1..=5`.
  Every jump-capable hull requires JDC 1, so `0` is never a valid jump input.

## Impact

- **New code**: `POST /api/v1/route/range` handler, a fan-out service (no Dijkstra, no
  overlay), and request/response DTOs including the summary header. Routed in the app
  and documented via utoipa/Swagger UI like the existing endpoints.
- **New catalog surface**: none required — the fan-out reuses `HullCatalog::by_name`/
  `by_type_id`, `range::{effective_ly, radius_m2, ly_between}`, and the kd-tree.
- **Modified code**: blops `resolve_effective_ly` JDC bound (`0..=5` → `1..=5`), its
  handler comment, and the `blops-staging` spec line; one blops validation test updated.
- **Tests**: unit tests for the fan-out service (range geometry, highsec/self exclusion,
  empty-set-is-200, sorting, summary tally), handler validation tests (missing hull,
  missing/`0`/`>5` JDC), and a HURL integration test against the fixture SDE.
- **No new dependencies.**

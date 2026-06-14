## Why

Future jump and bridge routing features — black-ops staging first, then jump
freighters, titan bridges, and reachability fan-out — all need to know how far a
hull can jump. That distance is a fact the EVE SDE already carries (per-hull
`jumpDriveRange` plus the Jump Drive Calibration skill bonus), but the foundation
only parses the map files. This change imports that hull-range data as a shared,
mechanic-agnostic primitive so every later feature consumes exact SDE truth instead
of hardcoded numbers. It is the primitive only: no new endpoint, no blops-specific
logic.

## What Changes

- Extend the SDE pipeline to parse two additional SDE files, `types.jsonl` and
  `typeDogma.jsonl`, alongside the existing map files. Because `types.jsonl` is
  ~150 MB (every item in EVE), parsing **streams and filters during the read**,
  keeping only jump-capable hulls — never materialising the whole file.
- Introduce a **`HullCatalog`**: a lookup from hull name/typeID to its base jump
  range (light-years) and group, built by joining published `types` that carry a
  `jumpDriveRange` dogma attribute (attribute **867**) with their `typeDogma` rows.
  The catalog is mechanic-agnostic — it holds ranges and groups, no jump/bridge/
  conduit logic.
- Fold `HullCatalog` into the hot-reloadable graph data so it **rides the same
  atomic swap** as the map graph: map data and hull data always come from one SDE
  build, with no skew.
- Add **pure, unit-tested range math**: `effective_ly = base_ly × (1 + bonus ×
  jdc_level)` where the per-level bonus (the JDC skill's `jumpDriveRangeBonus`,
  attribute **870**) is read from the SDE, plus helpers to convert light-years to
  the squared-metre distances the spatial index uses. JDC is the only skill that
  affects jump range; this formula is universal across all jump-capable hulls.
- Add a **trimmed** `types`/`typeDogma` test fixture (a subset, unlike the
  deliberately-full map fixtures) into the existing on-disk cache fixture, and a
  committed, reusable **fixture-regeneration script** (fronted by a `just`
  recipe) that rebuilds the whole fixture at a single SDE build.
- Surface a catalog readiness signal (hull count) in the existing `/health`
  response so operators can confirm the catalog loaded.

No breaking changes: every addition is additive. The wire API is unchanged (no new
or modified endpoint behaviour beyond the additive `/health` field).

## Capabilities

### New Capabilities

- `hull-catalog`: Imports jump-capable hull range data from the SDE
  (`types` + `typeDogma`) into an in-memory catalog keyed by name and typeID, and
  provides the pure range math (JDC-adjusted effective light-years, and light-year
  ↔ squared-metre conversions) that future jump/bridge features consume. Includes
  the trimmed test fixture and the whole-fixture regeneration script.

### Modified Capabilities

- `sde-graph`: The SDE load/parse/cache pipeline now additionally acquires and
  parses `types.jsonl` and `typeDogma.jsonl`, and the hot-reload atomic swap now
  carries the `HullCatalog` together with the map graph (single build, no skew).
- `health-and-openapi`: The health response additionally reports the loaded hull
  count, confirming the catalog is populated.

## Impact

- **Code**: `src/sde/cache.rs` (`FILES`, ZIP extract, offline `GEODESIC_SDE_DIR`
  path — all already generic over `FILES`), `src/sde/types.rs` (new raw rows),
  `src/sde/parse.rs` (streaming hull parse + dogma join), `src/model.rs`
  (`HullCatalog`, fold into `GraphData`), `src/graph.rs` (build catalog alongside
  graph), a new range-math module, `src/handlers/health.rs` + health DTO (hull
  count). Catalog construction is mechanic-agnostic and reused by the follow-up
  `add-blops-staging` change.
- **SDE acquisition**: download/extract now includes two more files; live
  hot-reload picks them up automatically (no new operational mechanism).
- **Tests/fixtures**: trimmed `types`/`typeDogma` added to
  `tests/fixtures/sde/<build>/`; new regeneration script + `just update-fixtures`
  recipe. The deferred fixture CI chore will later schedule this script.
- **Dependencies**: none new expected (existing `serde`/`zip`/`reqwest`/`kiddo`
  cover it).
- **No** new endpoint, auth, rate limiting, or blops/JF-specific behaviour — those
  are out of scope and land in follow-up changes.

## 1. SDE acquisition: add the two type files

- [x] 1.1 Add `types.jsonl` and `typeDogma.jsonl` to the `FILES` const in `src/sde/cache.rs`; confirm ZIP extract, `build_files_present`, `load_build`, and the `GEODESIC_SDE_DIR` offline path all pick them up via the generic `FILES` handling
- [x] 1.2 Add a cache test asserting a build missing `types.jsonl` or `typeDogma.jsonl` is treated as incomplete (re-fetched, not loaded partially)

## 2. Raw deserialization types

- [x] 2.1 In `src/sde/types.rs`, add a lean raw row for `types.jsonl` (`_key`, `groupID`, `name.en`, `published`; ignore mass/portionSize) and a raw row for `typeDogma.jsonl` (`_key`, `dogmaAttributes: [{attributeID, value}]`)
- [x] 2.2 Add named constants for the verified attribute/type IDs: `ATTR_JUMP_DRIVE_RANGE = 867`, `ATTR_JUMP_DRIVE_RANGE_BONUS = 870`, `JDC_SKILL_TYPE_ID = 21611`, and the Black Ops `GROUP_ID = 898`

## 3. Streaming hull parse + dogma join

- [x] 3.1 In `src/sde/parse.rs`, add a streaming parser that reads `types.jsonl` line by line and retains only `published` rows (keep name, typeID, groupID), never materialising the whole file
- [x] 3.2 Add a streaming parser for `typeDogma.jsonl` that, for retained typeIDs, extracts attribute 867 (range) and the JDC skill's attribute 870 (bonus); join to produce `(typeID, name, groupID, base_ly)` only for types that actually have attribute 867
- [x] 3.3 Unit-test the parse + join with tiny inline JSONL (a jump hull with 867, a hull with 868-but-no-867 excluded, an unpublished hull excluded, the JDC skill's 870 read)

## 4. HullCatalog data model

- [x] 4.1 In `src/model.rs`, define `HullCatalog` with `by_name` (ASCII-lowercased) and `by_type_id` lookups to `(base_ly, group_id)`, plus the JDC `bonus_per_level` read from attribute 870
- [x] 4.2 Add a method returning the minimum `base_ly` over a given `groupID`, computed at construction (used later for the worst-blops default)
- [x] 4.3 Add `HullCatalog` as a field of `GraphData`; update `build_graph_data` (and any constructors/tests) to build and carry it so it rides the same `ArcSwap` swap
- [x] 4.4 Wire catalog construction into the SDE load path so cold start, warm start, and hot-reload all populate it from the same build as the map graph

## 5. Range math module

- [x] 5.1 Add a pure range-math module: `LY_IN_METERS = 9.4607e15` const; `effective_ly(base_ly, bonus_per_level, jdc_level)` with `jdc_level` defaulting to 5; `radius_m2(ly)` and `ly_between(coords_a, coords_b)` (squared-metre helpers)
- [x] 5.2 Unit-test: 4.0 LY base at JDC 5 with 20% bonus → 8.0 LY; default level is 5; `radius_m2`/`ly_between` round-trip at the boundary using the same constant

## 6. Health endpoint: hull count

- [x] 6.1 Add `hull_count` to the health DTO and populate it from the loaded catalog in `src/handlers/health.rs`
- [x] 6.2 Update the health unit/integration test to assert a non-zero `hull_count` when the catalog is loaded

## 7. Test fixtures

- [x] 7.1 Generate a trimmed `types.jsonl` + `typeDogma.jsonl` (the jump-capable hulls + JDC skill rows) and add them to the existing `tests/fixtures/sde/<build>/` dir alongside the full map files
- [x] 7.2 Update fixture metadata/`build_files_present` expectations if needed so the offline cache fixture loads the four-file build cleanly
- [x] 7.3 Add a catalog integration test over the fixture: Black Ops group (898) min base range falls in a sane band (≈ 4 LY, not an exact pin); a known hull resolves by name and typeID

## 8. Fixture regeneration script

- [x] 8.1 Add a committed, reusable regeneration script (in `tests/fixtures/` or `scripts/`, `run-hurl.sh`-style header) that downloads the latest SDE build, extracts the map files verbatim and trims the type files, writing one new `<build>/` dir + updated `latest.jsonl`/`metadata.json` at a single build number
- [x] 8.2 Make the script's trim predicate the same rule as the catalog parser (published + attribute 867, plus JDC skill typeID 21611 explicitly); add a comment documenting that coupling
- [x] 8.3 Add a `just update-fixtures` recipe invoking the script

## 9. Verification

- [x] 9.1 Run `just check` (fmt, clippy -D warnings, tests, HURL) and confirm green
- [x] 9.2 Confirm `GraphData` foundation invariants (node-index ordering) still hold and existing routing/health tests pass unchanged

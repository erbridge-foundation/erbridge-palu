## 1. Prerequisite

- [ ] 1.1 Confirm `add-jump-hull-catalog` is implemented: `HullCatalog` (with min-over-group lookup) is in `GraphData` and the range math (`effective_ly`, `radius_m2`, `ly_between`) is available

## 2. DTOs and error

- [ ] 2.1 Add `ShipRef` to `src/dto.rs` — an untagged name-or-id enum mirroring `SystemRef` — resolving against the catalog's name/typeID lookups
- [ ] 2.2 Add the blops request DTO: `from`, `to` (`SystemRef`), optional `ship` (`ShipRef`), optional `jdc_level` (default 5), plus the existing routing knobs (preference, avoid, wormhole overlay fields)
- [ ] 2.3 Add the blops response DTOs: `chosen { gate_path, gate_jumps, bridge{ from, to, ly, in_range } }`, `alternates[ { system, gate_jumps, ly_to_B } ]`, and echoed `jdc_level`, `effective_ly`, `defaulted`
- [ ] 2.4 Add a distinct `AppError` variant for a highsec target B (cyno-impossible), mapped to a clear 4xx in `src/error.rs`; reuse the existing unknown-resource error for unknown system/hull
- [ ] 2.5 Unit-test DTO (de)serialization: `ShipRef` name vs numeric id; request defaults (jdc_level 5, no ship)

## 3. Multi-target shortest path

- [ ] 3.1 In `src/routing.rs`, add a multi-target Dijkstra variant that, given a source and a set of target nodes, settles the gate distance (and predecessor chain) to all targets in one pass over the gate graph, reusing the thread-local scratch
- [ ] 3.2 Have it honour the `RouteContext` overlay (avoid, preference weights, wormhole connections) identically to the single-target search
- [ ] 3.3 Unit-test: multiple targets at varied gate distances all get correct distances in one run; unreachable targets are reported as such; reachable subset is correct when some targets are blocked

## 4. Parameterized staging service

- [ ] 4.1 Add a staging service in `src/services/` that takes A, B, an effective range (LY), and security predicates (a "B must satisfy" check + the K-space ★ acceptance) — blops-agnostic, no hardcoded sec class
- [ ] 4.2 Reject B up front when it fails the destination predicate (highsec for blops) with the distinct error
- [ ] 4.3 Run the kd-tree radius query around B using `radius_m2(effective_ly)`; collect in-range K-space candidates (kd-tree already excludes J-space)
- [ ] 4.4 Run the multi-target Dijkstra from A over the candidates via `RouteContext`; rank by (gate_jumps, then `ly_to_B`); pick ★ = head, reconstruct the A→★ path, attach the bridge leg
- [ ] 4.5 Build the response with chosen + N fallback candidates; return the distinct no-candidate-in-range and ★-ungateable failures
- [ ] 4.6 Unit-test the service: ranking (fewest jumps, then closest); directional rule (highsec B rejected; highsec ★ accepted — assert they are separate predicates); no-candidate vs ungateable failures; fallbacks populated

## 5. Blops handler

- [ ] 5.1 Add the `POST /api/v1/route/blops` handler that resolves `from`/`to`/`ship`, applies blops defaulting (worst-blops hull = catalog min over group 898; jdc_level default 5; reject jdc_level outside 0..5), computes `effective_ly` via the range math, and calls the staging service with the blops predicates (B not highsec; ★ any K-space)
- [ ] 5.2 Echo `jdc_level`, `effective_ly`, and `defaulted` in the response; register the route in the router
- [ ] 5.3 Add the endpoint to the utoipa/OpenAPI docs with request/response schemas and an example; confirm it appears in Swagger UI

## 6. Integration and HURL tests

- [ ] 6.1 Pick a real in-fixture A/B/hull staging example reachable under the trimmed hull range + full SDE map; pin the expected ★, gate count, and bridge LY (in the style of the existing pinned route counts)
- [ ] 6.2 Add integration tests (handler→service over the fixture): happy-path staging, worst-hull default, highsec-B rejection, no-candidate-in-range, unknown hull
- [ ] 6.3 Add HURL contract tests for the new endpoint (happy path, highsec-B error, defaulting) to the offline HURL suite

## 7. Verification

- [ ] 7.1 Run `just check` (fmt, clippy -D warnings, tests, HURL) and confirm green
- [ ] 7.2 Confirm existing routing/health/wormhole tests and the gate-route endpoint behaviour are unchanged (additive only)

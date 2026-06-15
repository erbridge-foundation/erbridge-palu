## 1. DTOs

- [x] 1.1 Add `RangeRequest` DTO in `src/dto.rs`: `from: SystemRef`, `ship: ShipRef`
      (required, not `Option`), `jdc_level: u8` (required). No overlay fields.
- [x] 1.2 Add response DTOs: `RangeResponse` (`source`, `hull`, `jdc_level`,
      `effective_ly`, `summary`, `reachable`), `RangeHull` (`name`, `type_id`,
      `base_ly`), `RangeSummary` (`reachable_count`, `farthest_ly`, `by_sec_class`),
      and a reachable-system entry (reuse/mirror the `BlopsSystem` shape plus
      `jump_ly`). Derive `utoipa::ToSchema` on all, matching existing DTOs.

## 2. Reachability service

- [x] 2.1 Create `src/services/range.rs` with `compute_reachable(graph, source_node,
      effective_ly) -> RangeResponse` (mechanic-agnostic; no overlay, no Dijkstra).
- [x] 2.2 Run the kd-tree radius query (`radius_m2(effective_ly)`,
      `within_unsorted::<SquaredEuclidean>`) around the source coords.
- [x] 2.3 Filter the hits: drop the source node itself and drop highsec systems
      (J-space already excluded by the kd-tree).
- [x] 2.4 Map survivors to system entries with `jump_ly = round2(ly_between(...))`,
      sort ascending by full-precision light-year distance.
- [x] 2.5 Build the summary: `reachable_count`, `farthest_ly` (rounded), and the
      `by_sec_class` tally as a fold over the filtered set.
- [x] 2.6 Register the module in `src/services/mod.rs`.

## 3. Handler

- [x] 3.1 Add `route_range` handler in `src/handlers/route.rs`: load graph snapshot,
      resolve `from` (`resolve_system`), resolve `ship` against the catalog
      (reuse `resolve_hull`), validate `jdc_level` ∈ `1..=5`.
- [x] 3.2 Reject missing/unknown hull and `jdc_level` of `0` or `>5` with the existing
      `AppError` validation variants (no defaulting).
- [x] 3.3 Compute `effective_ly` via `range::effective_ly(base_ly, bonus_per_level,
      jdc_level)` and call `compute_reachable`; echo `jdc_level`, `effective_ly`, and
      the resolved hull into the response.
- [x] 3.4 Add the `#[utoipa::path]` annotation (tag `routing`, 200 + 400 responses)
      and register `POST /api/v1/route/range` in the router and OpenAPI doc.

## 4. Blops JDC validation fix

- [x] 4.1 In `src/handlers/route.rs`, change `resolve_effective_ly`'s bound from
      `jdc_level > 5` to reject `0` as well (`!(1..=5).contains(&jdc_level)`),
      updating the error message to state the `1..=5` range.
- [x] 4.2 Update the handler comment and any doc text referencing `0..=5` to `1..=5`.
- [x] 4.3 Update the blops validation test (`jdc_level_above_five_is_rejected` and/or
      add a `jdc_level_zero_is_rejected`) to assert `0` is now rejected.

## 5. Tests

- [x] 5.1 Service unit tests in `src/services/range.rs`: range geometry matches the
      catalog formula; highsec destinations excluded; source excluded; reachability
      ignores the gate graph (an ungateable in-range system is still listed); sorted
      ascending; summary count/farthest/by_sec_class correct.
- [x] 5.2 Service test: empty reachable set yields a `200`-shaped response (empty list,
      zero count) rather than an error.
- [x] 5.3 Handler validation unit tests: missing hull → 400, unknown hull → 400,
      `jdc_level` `0` → 400, `jdc_level` `>5` → 400, valid request resolves.
- [x] 5.4 HURL integration test under `tests/hurl/` against the fixture SDE: a known
      source + hull + JDC returns an expected reachable count and a pinned nearest
      system; assert highsec exclusion on a case that would otherwise include one.
- [x] 5.5 Run `cargo test` and the HURL suite; confirm the blops fix did not break the
      existing staging tests.

## 6. Docs

- [x] 6.1 Document `POST /api/v1/route/range` in the README alongside the other
      endpoints (request shape, required hull + JDC `1..=5`, 200-on-empty, no overlay).

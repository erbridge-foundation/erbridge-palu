## 1. DTOs and error plumbing

- [x] 1.1 Add a `pub(crate)` accessor on `AppError` exposing the `(code, message)`
      pair from the existing `parts()` so the per-destination error and
      `IntoResponse` share one source of truth (no new status logic).
- [x] 1.2 Reshape the request DTO in `dto.rs`: a shared header (`from`,
      `preference`, `avoid`, `use_wormholes`, `connections`, `include_*`) plus
      `to: Vec<SystemRef>`. Add a sanity-cap constant (1000).
- [x] 1.3 Add the response DTO: `{ from: <echoed once>, results: [...] }`, where
      each result echoes `to` as sent plus an untagged success (`jumps`/`path`) or
      failure (`error`/`message`) outcome. `ToSchema` derives; a concrete type
      alias for the `utoipa::path` `body =` annotation.
- [x] 1.4 Make `SystemRef` `Serialize` (echo `to`/`from` as-sent — bare string or
      number, original casing).
- [x] 1.5 Confirm the `utoipa` schema for the untagged result entry renders as a
      clean `oneOf`/`allOf`; record the outcome in design.md.
- [x] 1.6 DTO unit tests: fan-out request deserializes (header + `to` list);
      response echoes `from` once; success entry serializes as route + echoed `to`;
      failure entry as `to` + `error`/`message`; echoed key preserves as-sent
      name/id and casing.

## 2. Handler

- [x] 2.1 Reject an empty `to[]` and a `to[]` over the sanity cap with
      `AppError::InvalidParam` (request-level), before any route is computed.
- [x] 2.2 Rewrite `route_system`: load graph + EVE-Scout snapshots once; resolve
      the shared `from`, avoid set, and overlay once (a bad `from`/connection
      system is a request-level error); then map each destination through the
      existing `compute_gate_route` core, collecting `{ from, results }` (Ok →
      route + echoed `to`, Err → error + echoed `to`). `200` once past the header
      and `to`-bounds gates.
- [x] 2.3 Update the `#[utoipa::path]` annotation: fan-out request body, `200`
      returning the `{ from, results }` shape, `400` for the header/bounds tier;
      prose explaining per-destination errors.
- [x] 2.4 Handler unit tests: multi-destination in request order; bad shared `from`
      → `400`; per-destination unknown/unreachable → `200` with the error in its
      slot; empty `to[]` → `400`; over-cap → `400`; duplicate destinations answered
      positionally; shared overlay/snapshots resolved once.

## 3. HURL contract coverage

- [x] 3.1 Migrate the existing single-route requests in `route.hurl` and
      `wormhole_chain.hurl` to the fan-out shape (`to: ["X"]`, assert against
      `$.results[0]`), preserving their pinned jump-count facts.
- [x] 3.2 Add a multi-destination fan-out over the verified wormhole chain (shared
      header, several `to`): assert `from` echoed, `results` length, per-entry
      echoed `to`, and a known jump count on one entry.
- [x] 3.3 Add a mixed success/failure fan-out (good + unknown + unreachable
      destinations): assert `200`, the failing entries carry `error` and still echo
      their `to`, the others carry `path`.
- [x] 3.4 Add the request-level `400`s: bad shared `from`, empty `to[]`, and an
      over-cap `to[]` (body read from a committed fixture).

## 4. Integration test

- [x] 4.1 Mirror the `/system` integration tests for the fan-out: post a
      multi-destination body (incl. a deliberately failing destination and a
      bad-`from` request) and assert the `{ from, results }` shape and the
      request-level vs per-destination failure tiers.

## 5. Load testing (local only, not CI)

- [x] 5.1 Add the committed fan-out load body (`tests/load/fanout-wh.json`): one
      source, the verified ~29-edge wormhole chain in the shared header (stated
      once), and a diverse destination list (routes ~1..=39 jumps). Generate
      against the fixture and assert every destination resolves; share it with the
      multi-destination HURL test.
- [x] 5.2 Factor the fixture-boot scaffolding (background server, wait `/health`,
      teardown) shared by the HURL runner and the load-test recipes.
- [x] 5.3 Add `just load-test` driving `oha` in the **foreground** (live TUI;
      server backgrounded; no stdout pipe) against the fixture server with the
      shared body. Hint to `cargo install oha` if missing.
- [x] 5.4 Add `just load-test-plain` (same as 5.3 but `--no-tui` for a capturable
      plain-text summary).
- [x] 5.5 Confirm neither load recipe is wired into CI or the `check` recipe; add a
      recipe comment noting the hot-graph upper-bound caveat from design.md.

## 6. Verification

- [x] 6.1 Run `just check` (fmt, clippy, test, hurl) and confirm green.
- [x] 6.2 Manually exercise `just load-test` once to confirm the TUI renders
      through `just` and `just load-test-plain` produces a capturable summary.

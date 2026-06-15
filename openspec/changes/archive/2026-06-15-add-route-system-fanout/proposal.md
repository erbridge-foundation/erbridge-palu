## Why

The system-route endpoint answers one route per call, but its primary consumer is
a wormhole mapper that, from a pilot's selected system, displays routes to several
hubs at once ("from where I'm sitting, with my scanned chain, how do I reach Jita,
Amarr, Hek, Dodixie, Thera?"). That is one query, not N independent ones: the
**source, the wormhole chain, and the routing policy are all shared** — only the
**destination varies**. Issuing it as N calls (or as N batched requests that each
repeat the source and the whole chain) is wasteful: a 29-edge scanned chain would
be sent once per destination instead of once per query.

This change reshapes `/api/v1/route/system` to match the query's real structure: a
shared header (`from`, overlay, preference, avoid) plus a list of destinations,
returning one result per destination.

## What Changes

- **BREAKING (pre-prod)**: `POST /api/v1/route/system` now takes a **fan-out**
  request — a single shared header plus `to: [...]` (a list of destinations) — and
  returns `{ from, results: [...] }`. A single route becomes `to: ["X"]`. Not
  shipped to production, so taken now rather than carried as a second endpoint.
- The shared header carries everything that is the same for every destination:
  `from`, `preference`, `avoid[]`, `use_wormholes` + `connections[]`,
  `include_thera` / `include_turnur` / `include_zarzakh`. The wormhole chain is
  stated **once**.
- Each result echoes the `to` it answered (exactly as sent) plus either the route
  (`jumps`/`path`) or `error`/`message` — the same code/message a single-request
  `4xx` would carry. `from` is echoed once at the top level.
- **Failure tiers mirror the request's structure.** A problem in the shared header
  (unresolvable `from`, an unknown connection system, a bad `preference`) fails the
  **whole request** with a `400`, because that state is shared and cannot
  meaningfully be reported per destination. A problem with one destination
  (unknown or unreachable) is reported **in that destination's slot** while the
  request still returns `200`. So one bad hub never sinks the routes that resolved.
- `to[]` must be non-empty (an empty list is a malformed query → `400`) and is
  bounded by a **sanity cap of 1000** (`>1000` → `400`). The cap is a guard against
  a runaway/accidental single request, **not** a workload limit — any real fan-out
  is far smaller. Abuse/DoS is out of scope here; if the service is ever exposed
  publicly it gains API-key auth + tower rate limiting at the edge, which own that
  boundary (the per-request cap stays useful regardless, since rate limiting bounds
  frequency, not the size of one request).
- Duplicate destinations are allowed and answered positionally; the shared overlay
  is **not** echoed back (only `from`), to avoid returning the chain the client
  just sent.
- Add HURL contract coverage for the fan-out, including a many-destination request
  over the verified wormhole chain, a mixed success/failure fan-out, the
  empty-`to[]` and over-cap `400` boundaries, and the single-destination case.
- Add a local-only **`just load-test`** recipe (and a plain-text sibling) driven by
  [`oha`](https://github.com/hatoo/oha) against the fixture-booted server, firing a
  committed fan-out body: one source, the shared 29-edge wormhole chain stated
  once, and a diverse destination list (routes spanning ~1..=39 jumps). **Not run
  in CI.**

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `gate-routing`: The "System route endpoint" requirement changes from a single
  `from`/`to` route to a fan-out: a shared header plus a destination list,
  returning `{ from, results: [...] }` with per-destination results. New scenarios
  cover multi-destination success, per-destination failure (200 with an in-slot
  error), a bad shared `from` (request-level 400), the empty-`to[]` 400, and the
  over-cap 400.

## Impact

- **Modified code**: `route_system` handler (resolve the shared header once,
  resolve the overlay once, map each destination through the existing
  `compute_gate_route` core, collect into `{ from, results }`); `dto.rs` (new
  fan-out request with a shared header + `to: Vec<SystemRef>`, response header +
  per-destination result element echoing `to`); a small `pub(crate)` accessor on
  `AppError` so the per-destination error and the existing `IntoResponse` share one
  source of truth for `code`/`message`. The sanity cap and empty-`to[]` guard are
  new `AppError::InvalidParam` cases.
- **Unchanged**: the routing core (`compute_gate_route`) and router wiring — the
  handler resolves to the same path, so the OpenAPI path-collection tests are
  unaffected. The fan-out is **routing-specific** (it bakes in "one source"), so it
  is deliberately not generalised to blops/range.
- **Tests**: DTO serde (fan-out request; header + result serialization; echoed `to`
  as-sent), handler unit tests (multi-destination in order; bad-`from` 400;
  per-destination failure at 200; empty-`to[]` and over-cap 400; overlay/snapshots
  resolved once), integration + HURL coverage as listed above.
- **Tooling (not CI)**: `oha` as an opt-in dev tool (`cargo install oha`); the load
  body collapses the wormhole chain to a single shared header + a destination list
  (smaller and more faithful than repeating it), shared with the many-destination
  HURL test. Load numbers documented as a hot-graph upper bound.
- **No new build/runtime dependencies.**

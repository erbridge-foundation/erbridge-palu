---
name: palu-backend
description: |
  Rules for the palu Rust REST API: layered architecture (handler → service →
  in-memory graph/SDE), DTOs with utoipa schemas, bespoke JSON responses,
  AppError → HTTP mapping, and full test coverage (unit + integration + HURL).
  This service has NO database — data is the in-memory SDE graph and EVE-Scout
  snapshot, both behind `ArcSwap`. There is NO auth: it is docker-internal and
  unauthenticated.
  TRIGGER when: starting work on any Rust task in this repo, including scaffolding
  before files exist; editing files under src/{handlers,services}/, src/{routing,
  range,graph,model,sde,eve_scout}.rs, or src/{dto,error,app_state,config,tasks}.rs;
  writing or modifying axum code; adding HURL tests under tests/hurl/; touching
  tests/integration.rs; reviewing a backend PR; designing a new endpoint or a new
  routing/graph function; applying tasks from an OpenSpec change whose tasks.md
  mentions Rust files. Invoke before writing the first line of Rust in a session.
  SKIP: infrastructure-only changes (Dockerfile, docker-compose, Traefik/CI
  config), or documentation changes that don't touch handler/service/graph code.
---

# palu Rust REST API — Rules & Guidance

palu is an EVE Online gate-routing API. It loads CCP's Static Data Export (SDE)
into an in-memory graph and answers routing queries over it. **There is no
database and no authentication.** The crate lives at the repo root (binary
`erbridge-palu`), not under `backend/`.

## Module Layout

Code is organised by **layer**, then by domain within a layer. HTTP handlers
live in `src/handlers/`; business logic in `src/services/`. The "data layer" is
not a database — it is the in-memory SDE/graph loaders and the routing
algorithms at the crate root.

```
src/
├── main.rs            # entry point: load SDE, build graph, spawn pollers, serve
├── lib.rs            # build_router() + the utoipa ApiDoc (OpenAPI document)
├── app_state.rs      # AppState: two ArcSwaps (graph + eve_scout) + last_reload_at
├── config.rs         # env-var loading (PALU_*), tracing init, version/commit
├── error.rs          # AppError enum + IntoResponse ({ "error", "message" })
├── dto.rs            # ALL wire DTOs in one file, with serde + utoipa::ToSchema
├── model.rs          # internal domain types: System, GraphData, HullCatalog, …
├── tasks.rs          # background pollers: SDE hot-reload + EVE-Scout refresh
│
├── handlers/         # HANDLER LAYER — HTTP boundary, one file per route group
│   ├── mod.rs
│   ├── health.rs     # GET /health — flat status doc, no auth
│   └── route.rs      # POST /api/v1/route/{system,blops,range}
│
├── services/         # SERVICE LAYER — business logic, one file per domain
│   ├── mod.rs
│   ├── route.rs      # gate routing: resolve endpoints, overlay, Dijkstra, shape
│   ├── blops.rs      # black-ops staging solver
│   └── range.rs      # jump-range reachability fan-out
│
├── sde/              # DATA LOADERS — SDE disk cache + parse (not a "db")
│   ├── mod.rs
│   ├── cache.rs      # manifest fetch, download/extract, metadata, freshness
│   ├── parse.rs      # JSONL stream parsers (systems, gates, hull catalog)
│   └── types.rs      # raw deserialise structs for the SDE files
│
├── graph.rs          # build_graph_data(): RawSdeData → GraphData (petgraph + kd-tree)
├── routing.rs        # Dijkstra + per-request overlay (Preference, RouteContext)
├── range.rs          # jump-range math (effective_ly, JDC bonus)
└── eve_scout.rs      # EVE-Scout HTTP client + EveScoutSnapshot (Thera/Turnur sigs)
```

**Rules enforced by this layout:**
- Never add a handler function outside `src/handlers/`.
- Never add a service function outside `src/services/`.
- Handlers own the HTTP boundary only: load snapshots from `AppState`, call a
  service, map the result/`AppError` to a response. No routing or graph logic.
- Services own all business logic and never import `axum` or HTTP types.
- The in-memory data layer (`sde/`, `graph`, `model`, `routing`, `range`,
  `eve_scout`) is consumed by services; it must not import from `handlers` or
  `services`.
- Wire DTOs all live in `src/dto.rs`. Internal domain types live in
  `src/model.rs` and are never serialised directly (see DTOs, below).

There is no `db/` layer, no `dto/` directory, no cross-cutting SQL modules — if
the upstream template mentions any of those, it does not apply here.

---

## Architecture: The Only Permitted Flow

```
HTTP Request
    │
    ▼
Handler  (src/handlers/)
    │  load ArcSwap snapshots, call one service fn, return a DTO (or AppError)
    ▼
Service  (src/services/)
    │  resolve endpoints, assemble overlay, run routing, shape the DTO
    ▼
Graph / SDE / routing  (graph.rs, routing.rs, range.rs, model.rs, sde/, eve_scout.rs)
    │  pure-ish computation over the in-memory GraphData + EveScoutSnapshot
    ▼
In-memory data (ArcSwap<GraphData>, ArcSwap<EveScoutSnapshot>)
```

**This flow is strictly one-directional and may not be broken:**

- Handlers **must not** reach into graph/routing internals directly — they go
  through a service. (Loading `state.graph.load()` / `state.eve_scout.load()`
  and handing the snapshot to a service is the handler's job; computing on it is
  not.)
- Services **must not** import `axum` or any HTTP framework types.
- The data layer (`graph`, `routing`, `model`, `sde`, `eve_scout`) **must not**
  import from `handlers` or `services`.
- No layer may import from a layer above it.

This codebase has no mechanical `layering.rs` guard test (unlike the upstream
template). The boundary is maintained by review and by the module doc-comments
that state each layer's contract — keep those accurate. If you find yourself
wanting `axum::` inside `services/`, or `state.graph` math inside a handler,
that's the smell.

---

## Handler Rules

- Handlers live in `src/handlers/`.
- Each handler **must** accept injected state via `State<AppState>` — no globals.
- A handler loads the snapshots it needs **once** at the top
  (`let graph = state.graph.load();`) so the whole computation sees a consistent
  view, then calls **one** service function for the logical operation.
- Handlers **must** return a DTO (never a `model.rs` type), as
  `Json<SomeDto>` or `Result<Json<SomeDto>, AppError>`.
- Every handler carries a `#[utoipa::path(...)]` annotation describing its
  route, request body, responses, and tag — this is how the OpenAPI doc is
  generated (see OpenAPI, below). A handler without it won't appear in the spec.
- Request-body validation that is cheap and HTTP-shaped (presence, obvious
  bounds) can live in the handler; validation that needs the graph/catalog
  (resolving a system name, checking a hull exists) belongs in the service and
  surfaces as an `AppError`.

```rust
// CORRECT — load snapshots once, delegate to one service fn, return a DTO
#[utoipa::path(
    post,
    path = "/api/v1/route/system",
    request_body = GateRouteRequest,
    responses((status = 200, body = GateRouteResponse), (status = 400, ...)),
    tag = "routing",
)]
pub async fn route_system(
    State(state): State<AppState>,
    Json(req): Json<GateRouteRequest>,
) -> Result<Json<GateRouteResponse>, AppError> {
    let graph = state.graph.load();
    let scout = state.eve_scout.load();
    let resp = compute_gate_route(&graph, &scout, &req)?;
    Ok(Json(resp))
}

// WRONG — handler computing on the graph itself
pub async fn route_system(State(state): State<AppState>, ...) {
    let graph = state.graph.load();
    let path = routing::shortest_path(&graph.gate_graph, ...); // ❌ service's job
}
```

---

## Service Rules

- Services live in `src/services/`, one file per domain (`route`, `blops`,
  `range`).
- Services **must not** import `axum` or any HTTP framework types. They take the
  loaded snapshots (`&GraphData`, `&EveScoutSnapshot`) and the request DTO as
  borrowed inputs, and return a response DTO or `AppError`.
- Services own all business logic: resolving system/hull references against the
  catalog, assembling the per-request overlay (avoid set + wormhole edges),
  driving Dijkstra, and shaping the result DTO.
- Shared routing inputs are projected into a common view (e.g. `OverlayInputs`)
  so overlay/avoid-set assembly lives in one place rather than being duplicated
  per endpoint. When two endpoints need the same preparation, factor it into a
  shared helper rather than copying it.
- Services return DTOs (or domain results) and `AppError` — never raw HTTP
  responses or status codes. The `AppError` → status mapping is owned entirely
  by `error.rs`'s `IntoResponse`.

```rust
// CORRECT — no axum, borrowed snapshots in, DTO + AppError out
pub fn compute_gate_route(
    graph: &GraphData,
    scout: &EveScoutSnapshot,
    req: &GateRouteRequest,
) -> Result<GateRouteResponse, AppError> {
    let from = resolve_system(graph, &req.from)?; // AppError::UnknownSystem on miss
    // … assemble overlay, run routing, shape DTO …
}
```

---

## DTOs

- **All wire DTOs live in `src/dto.rs`** (request and response types together).
  Do not scatter them per-handler and do not put them in `model.rs`.
- Every DTO derives `serde` (`Deserialize` for requests, `Serialize` for
  responses) **and** `utoipa::ToSchema`, with `#[schema(example = ...)]` on
  notable fields so the OpenAPI doc and Swagger UI are useful.
- A response DTO is an **explicit allowlist** of fields that are safe and
  intended to cross the wire. Treat every field as a deliberate exposure
  decision.
- **Never `#[derive(Serialize)]` on a `model.rs` type.** Internal domain types
  (`System`, `GraphData`, `HullEntry`, …) are not `Serialize` — serialisation
  is a DTO responsibility. Services build the DTO explicitly from the domain
  values; do not smuggle a domain struct onto the wire.
- Never use `#[serde(flatten)]` to fold a domain/model type into a DTO — it
  drags every present and future field of that type into the response.
- For client-facing reference types that accept either a name or an id, use the
  established `#[serde(untagged)]` enum pattern (`SystemRef`, `ShipRef`): a
  quoted value is a name, a JSON number is an id.
- When converting between a DTO and a domain enum is non-trivial, use a `From`
  impl in `dto.rs` (e.g. `impl From<RoutePreference> for Preference`) rather
  than mapping inline at every call site.

```rust
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(untagged)]
pub enum SystemRef {
    #[schema(example = "Jita")]
    Name(String),
    #[schema(example = 30000142)]
    Id(i64),
}
```

---

## Response Shapes (no universal envelope)

palu does **not** use a `{ "data": … }` envelope. Each endpoint returns a
bespoke DTO shaped for its operation. Do not introduce a generic `ApiResponse<T>`
wrapper.

- **`GET /health`** returns a flat status document (no auth, no wrapper):
  `{ "status": "ok", "app_version", "git_commit", "sde_version", "systems",
  "edges", "hull_count", "last_sde_reload_at", "sig_count",
  "last_evescout_fetch_at" }`. The route is **`/health`**, not `/api/health`.
- **`POST /api/v1/route/system`** (fan-out) returns `{ "from", "results" }`,
  one entry per destination in request order. A shared-header problem is a
  request-level `400`; a per-destination problem is reported **in that
  destination's result entry** as an `error`/`message` pair while the request
  still returns `200`, so one bad destination doesn't sink the others.
- **`POST /api/v1/route/blops`** and **`/api/v1/route/range`** return their own
  result DTOs (`BlopsRouteResponse`, `RangeResponse`).
- **Errors** (from `AppError::into_response`) are
  `{ "error": "<code>", "message": "<human text>" }` with the mapped status.
  This same `(code, message)` pair is what the fan-out embeds per-destination —
  `AppError::code_message()` is the single source of truth so an in-slot error
  and a standalone `4xx` body cannot drift.

---

## Error Handling

- A single `AppError` enum lives in `src/error.rs`; its `IntoResponse` impl owns
  the `(StatusCode, code, message)` mapping via the private `parts()` method.
  Handlers and services **never** construct a `StatusCode` directly.
- **Services return `AppError` directly.** There is no separate `ServiceError`
  type — at this codebase's size that would be pure ceremony. The rule that
  matters is behavioural: services never build HTTP responses; they return
  `AppError` variants and `IntoResponse` maps them.
- Add new variants to `AppError` (with a distinct error `code` string and the
  right status) rather than reaching for an ad-hoc response. Keep the
  domain-specific variants meaningful — e.g. `CynoTargetHighsec` is a distinct
  `400` from `NoStagingInRange`'s `404` because the *cause* differs.
- The fan-out path embeds a per-destination failure using
  `AppError::code_message()` so the in-body error and the standalone error body
  share one source of truth. If you add a variant, that consistency is automatic
  — don't hand-roll a parallel string.
- **Never use `.unwrap()` or `.expect()` in non-test code.** Enforced:
  `[lints.clippy] unwrap_used / expect_used = "warn"` in `Cargo.toml`, and CI
  runs clippy with `-D warnings` so they are effectively denies. `clippy.toml`
  allows them in tests, and `tests/*.rs` carry crate-level allows. A
  provably-infallible case in non-test code may carry a narrowly-scoped
  `#[allow(clippy::expect_used)]` **with a comment proving why it cannot panic**
  — an allow without that proof is a review-blocker.

---

## OpenAPI / Swagger

The OpenAPI document is generated from handler annotations — there is no manual
path registry to drift.

- The top-level `ApiDoc` (`#[derive(OpenApi)]` in `lib.rs`) declares only
  `info` and `tags`. Paths are collected by `OpenApiRouter` in `build_router`
  via `routes!(...)`, then `.split_for_parts()`.
- **Adding an endpoint means three coordinated edits:** the `#[utoipa::path]`
  annotation on the handler, a `.routes(routes!(handlers::…::fn))` line in
  `build_router`, and the request/response DTOs deriving `ToSchema`. Miss the
  `routes!` line and the path is absent from the spec; the `lib.rs` tests assert
  every expected path is present, so keep them in lockstep.
- Swagger UI is served at `/swagger-ui` and the JSON at
  `/api-docs/openapi.json`, unconditionally (the service is docker-internal).

---

## Testing Requirements

There is **no database**, so there is no `sqlx`, no `#[sqlx::test]`, no `.sqlx`
offline cache, and no per-test database. Tests run against the in-memory graph,
loaded either from a tiny inline `RawSdeData` or from the committed real-SDE
fixture under `tests/fixtures/sde/`. External HTTP (EVE-Scout) is never hit in
tests — the snapshot is injected directly into `AppState`.

### Unit Tests — cover every non-trivial function

Every function with meaningful behaviour gets a unit test — handlers, services,
parsers, graph builders, routing helpers, error mapping, and pure math alike.
The only exclusions are trivial glue (a one-line `From`, a constructor that just
assigns fields, a handler that does nothing but `service.call()`). A branch, a
transformation, a validation, or an error path needs a test.

Tests live in `#[cfg(test)] mod tests` **within the file they cover** (the
established pattern across the crate — see `error.rs`, `app_state.rs`,
`handlers/health.rs`, `sde/cache.rs`).

**Patterns to use:**

| What to test | How |
|---|---|
| Handler shape / DTO output | construct `AppState::new(Arc::new(build_graph_data(raw, build)))`, `await` the handler fn directly, assert on the returned `Json<Dto>` |
| Service logic (routing, overlay, staging, range) | build a small `GraphData` (or use the fixture), call the service fn, assert on the DTO / `AppError` |
| `AppError` → response mapping | construct the variant, call `.into_response()`, assert status + `{ error, message }` body (see `error.rs` tests) |
| SDE parse / cache | build minimal JSONL or a synthetic ZIP in a `tempfile::tempdir()`, parse/extract, assert (see `sde/cache.rs` tests) |
| Pure math / helpers (`effective_ly`, sec-class) | direct calls, every branch |

**Inline-graph handler test (no DB, no mocks):**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::graph::build_graph_data;
    use crate::model::RawSdeData;

    #[tokio::test]
    async fn health_reports_loaded_build() {
        let raw = RawSdeData { systems: vec![], gate_pairs: vec![], hulls: Default::default() };
        let state = AppState::new(Arc::new(build_graph_data(raw, 7)));
        let Json(body) = health(State(state)).await;
        assert_eq!(body.sde_version, 7);
    }
}
```

**Error-mapping test:**

```rust
#[tokio::test]
async fn unknown_system_maps_to_400() {
    let resp = AppError::UnknownSystem("Nowhere".into()).into_response();
    // assert status == 400 and body["error"] == "unknown_system"
}
```

Do **not** introduce `mockall` or trait doubles — the in-memory graph is cheap
to build for real, so test against a real (small or fixture) graph.

### Integration Tests — handler→service→graph, end-to-end

- Live in `tests/integration.rs` at the repo root.
- Build the real router with `build_router(state)` over a state loaded from the
  committed SDE fixture (`tests/fixtures/sde/`, loaded via `SdeCache` exactly
  like production), and drive it with real requests via
  `tower::ServiceExt::oneshot`.
- Inject the EVE-Scout snapshot directly into `AppState` — never call the live
  EVE-Scout API from a test.
- Cover every endpoint at least once, happy path plus key error paths
  (unknown system, unreachable, invalid param, per-destination in-slot error).
- The fixture has known reference facts (e.g. Jita → Amarr is 11 gate jumps;
  J-space systems are gateless and unreachable without a wormhole overlay) — use
  them as assertions and keep the doc-comment at the top of `integration.rs`
  accurate if the fixture is regenerated.

### HURL Tests — the HTTP contract

- Every endpoint has at least one HURL test in `tests/hurl/`, named after the
  resource: `route.hurl`, `blops.hurl`, `range.hurl`, `health.hurl`,
  `openapi.hurl`.
- HURL tests are the source of truth for the over-the-wire contract: status
  codes, headers, and response shape. Assert the **bespoke** shape (no `data`
  envelope) — e.g. `$.from` / `$.results` for the fan-out, `$.status == "ok"`
  for health.
- Run them via `tests/hurl/run-hurl.sh` against a server booted with the offline
  fixture (`tests/fixtures/boot-server.sh` / `PALU_SDE_DIR`).

`tests/hurl/health.hurl` — flat health doc, no envelope:

```hurl
GET http://localhost:8080/health

HTTP 200
[Asserts]
jsonpath "$.status" == "ok"
jsonpath "$" not exists "data"
```

Two HURL requests in one file are separated by a blank line; do not put a `---`
separator inside a `.hurl` file.

---

## Checklist Before Committing

- [ ] Handler loads snapshots once, calls exactly one service fn, returns a DTO
- [ ] Handler does no routing/graph computation (that's the service's job)
- [ ] Service imports no `axum`/HTTP types; returns DTO + `AppError`
- [ ] No `model.rs` type derives `Serialize` or crosses the wire; DTOs are in `dto.rs`
- [ ] Response is a bespoke DTO (no `ApiResponse`/`data` envelope); errors are `{ error, message }`
- [ ] New endpoint: `#[utoipa::path]` + `routes!()` in `build_router` + `ToSchema` DTOs, and the `lib.rs` path-presence test updated
- [ ] New `AppError` variant has a distinct `code` and correct status in `parts()`
- [ ] No `.unwrap()` / `.expect()` in non-test code; any `#[allow]` carries a cannot-panic proof comment
- [ ] Unit test for every non-trivial function (inline `#[cfg(test)]`, real in-memory graph, no mocks)
- [ ] Integration test in `tests/integration.rs` for every endpoint (happy + key error paths)
- [ ] HURL test in `tests/hurl/` for every endpoint, asserting the real (envelope-free) shape
- [ ] `cargo fmt` run
- [ ] `cargo clippy --all-targets -- -D warnings` passes
```
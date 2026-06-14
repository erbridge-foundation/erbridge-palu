## Why

`erbridge-geodesic` is a rewrite of a prior EVE Online routing service. We need a
solid foundation: load the Static Data Export (SDE), build the New Eden gate graph
in memory, and serve standard gate routing over HTTP. This foundation must be
designed so that the planned enhancements — black ops staging (LY/jump-range
queries), wormhole ship-size fit filtering, and auth — land as *additive*,
non-breaking changes rather than rewrites. This initial version runs entirely
inside a docker-compose network, so it ships with **no authentication**.

## What Changes

- Load the SDE (`mapSolarSystems.jsonl`, `mapStargates.jsonl`) from CCP's
  build-number manifest, cache it on disk, parse it, and build an in-memory
  `GraphData` (~2.4 MB: systems, undirected gate graph, id/name lookups, and a
  kd-tree spatial index).
- Hold `GraphData` behind an `ArcSwap` and run a **background hot-reload task**
  that polls the SDE manifest and atomically swaps in a freshly-built graph when a
  newer build appears. Reload failures are non-fatal (keep serving the current
  graph); only the initial load is fatal.
- Build a **kd-tree spatial index** over K-space systems now, even though nothing
  queries it yet. It rides the graph-construction/reload path so the future black
  ops change is pure query logic with zero churn to graph construction. (~0.5 MB.)
- Serve `POST /route/gate`: Dijkstra over the gate graph with three preferences —
  `shortest`, `safest` (security-weighted), and `prefer_gates` (small additive
  penalty per wormhole hop, so wormholes are taken only when they shorten the
  route).
- Per-request graph overlay (`RouteContext`): an **avoid set** (user `avoid[]`
  ∪ Zarzakh unless opted in) subtracted, and **wormhole edges** added from two
  sources via the same mechanism: user `connections[]` and EVE-Scout
  Thera/Turnur signatures.
- Fetch **EVE-Scout** Thera/Turnur signatures via a second background-cached
  `ArcSwap` poller (no per-request network). `include_thera` / `include_turnur`
  filter this snapshot into the overlay; expired signatures are dropped at read
  time.
- Exclude **Zarzakh** (30100000) from transit by default (its gate-lock mechanic
  makes transit routes a lie); an explicit `include_zarzakh` flag (default
  `false`) lets the caller opt in and own the 6-hour-lock decision in the UI.
- Reserve forward-compatible optional schema: each wormhole entry carries an
  optional `max_size` (parsed, **not yet enforced**) so the future ship-fit change
  is purely behavioral.
- Serve `GET /health` with build number, system/edge counts, `last_reload_at`, and
  EVE-Scout freshness (`sig_count`, `last_fetch_at`).
- Generate an **OpenAPI 3.1** document from the handlers/DTOs via `utoipa` +
  `utoipa-axum`, and serve an interactive **Swagger UI** (always on, since the
  service is docker-internal).
- **No auth, no rate limiting, no hot-reload of secrets** in this foundation.
  Black ops staging, JDC/jump-range math, wormhole ship-fit filtering, and auth
  are explicitly **out of scope** and deferred to future changes.

## Capabilities

### New Capabilities

- `sde-graph`: Load/cache the SDE, build the in-memory gate graph + kd-tree, and
  hot-reload it behind an `ArcSwap` on a background poll.
- `gate-routing`: `POST /route/gate` — Dijkstra with `shortest` / `safest` /
  `prefer_gates` preferences over a per-request overlay (avoid set + wormhole
  edges).
- `wormhole-overlay`: Per-request wormhole edges from user `connections[]` and
  from background-cached EVE-Scout Thera/Turnur signatures, with reserved (unused)
  `max_size` and Zarzakh opt-in semantics.
- `health-and-openapi`: `GET /health`, OpenAPI 3.1 generation, and Swagger UI.

### Modified Capabilities

(none — greenfield foundation; no existing specs)

## Impact

- **New code**: SDE loader/cache, graph + kd-tree construction, `ArcSwap` state,
  two background poll tasks (SDE manifest, EVE-Scout), axum app, `RouteContext` +
  Dijkstra, DTOs with `utoipa` derives.
- **Dependencies** (new): `axum`, `tokio`, `tower`/`tower-http`, `petgraph`,
  `kiddo`, `arc-swap`, `reqwest`, `serde`/`serde_json`, `rustc-hash`, `zip`,
  `directories`, `tracing`/`tracing-subscriber`, `thiserror`/`anyhow`, `smallvec`,
  `utoipa`, `utoipa-axum`, `utoipa-swagger-ui`.
- **External APIs consumed**: CCP SDE manifest + per-build ZIP; EVE-Scout
  `GET https://api.eve-scout.com/v2/public/signatures`.
- **Runtime**: single docker-compose service, no auth, `GEODESIC_*` env vars,
  ~2.4 MB live graph / <50 MB RSS.
- **Deliberately NOT touched**: black ops staging, JDC, wormhole ship-fit
  enforcement, authentication, rate limiting.

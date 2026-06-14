# Tasks — Gate Routing Foundation

## 1. Scaffolding

- [x] 1.1 Add dependencies to `Cargo.toml` (axum, tokio, tower/tower-http, petgraph, kiddo, arc-swap, reqwest, serde/serde_json, rustc-hash, zip, directories, tracing/tracing-subscriber, thiserror/anyhow, smallvec, utoipa, utoipa-axum, utoipa-swagger-ui)
- [x] 1.2 Set up tracing/tracing-subscriber init and `GEODESIC_*` env-var helpers (port, cache dir, intervals)
- [x] 1.3 Stand up the axum app + listener (default `0.0.0.0:5001`, `GEODESIC_PORT` override) with a placeholder router

## 2. SDE loader and disk cache

- [x] 2.1 Resolve cache dir via `directories`, honour `GEODESIC_CACHE_DIR`
- [x] 2.2 Fetch the build-number manifest; compare against cached build
- [x] 2.3 Download + extract `mapSolarSystems.jsonl` / `mapStargates.jsonl` with ZIP-slip defence; write atomically (tmp + rename); maintain `metadata.json`; prune old builds
- [x] 2.4 Line-by-line JSONL parsing of the two files into row types
- [x] 2.5 Unit tests for cache atomicity, ZIP-slip rejection, and parsing against a small fixture

## 3. Graph construction

- [x] 3.1 Build `System` structs; derive `sec_class` (highsec iff non-WH and `securityStatus >= 0.45`) and `is_wormhole` (`region_id >= 11_000_000`)
- [x] 3.2 Build the undirected, deduplicated petgraph gate graph from directed stargate pairs
- [x] 3.3 Build id→index and case-insensitive name→index lookups
- [x] 3.4 Build the kd-tree spatial index over K-space positions (constructed, not yet queried)
- [x] 3.5 Assemble `GraphData`; unit tests for edge dedup, sec classification, and name-case lookup

## 4. Shared state and hot-reload

- [x] 4.1 Hold `GraphData` in `ArcSwap`; handlers load a per-request snapshot
- [x] 4.2 Background SDE poller (`GEODESIC_SDE_RELOAD_INTERVAL_SECS`, default 3600, 0 disables): build new graph fully in memory, then atomic swap
- [x] 4.3 Make reload failures non-fatal (log, keep current graph); keep initial load fatal
- [x] 4.4 Track `last_reload_at`; comment the transient memory-doubling during swap

## 5. EVE-Scout poller

- [x] 5.1 Define signature row types matching the EVE-Scout v2 response (out/in system, max_ship_size, signature_type, expires_at)
- [x] 5.2 Background poller (`GEODESIC_EVE_SCOUT_INTERVAL_SECS`, default 600, 0 disables) holding an `ArcSwap` snapshot; failures log and keep last snapshot
- [x] 5.3 Partition by origin: Thera (31000005) / Turnur (30002086); keep only `signature_type == wormhole`

## 6. Routing core

- [x] 6.1 `RouteContext` wrapping `&GraphData`: avoid set + wormhole-edge overlay, neighbour iterator that filters avoided nodes
- [x] 6.2 Composable edge weights: `shortest`, `safest` (security penalty), `prefer_gates` (small additive WH penalty); large finite penalties, never infinity
- [x] 6.3 Dijkstra with reusable thread-local scratch buffers and SmallVec paths; label steps start/stargate/wormhole
- [x] 6.4 Unit tests for each preference, avoid routing, and prefer_gates take/skip thresholds on a fixture

## 7. HTTP endpoints

- [x] 7.1 DTOs for `/route/gate` request/response with `utoipa` `ToSchema`/`IntoParams`; connection `max_size` optional (parsed, unused)
- [x] 7.2 `POST /route/gate` handler: resolve from/to, assemble avoid set (user avoid[] ∪ Zarzakh unless `include_zarzakh`), assemble overlay (connections + include_thera/turnur from snapshot, dropping expired sigs), run routing, map errors to HTTP codes
- [x] 7.3 `GET /health` (build number, system/edge counts, `last_reload_at`, EVE-Scout `sig_count`/`last_fetch_at`), no auth
- [x] 7.4 Wire OpenAPI via `utoipa-axum` `OpenApiRouter`; serve OpenAPI JSON + Swagger UI unconditionally

## 8. Integration & docs

- [x] 8.1 Integration tests against a cached SDE fixture: known route jump counts, avoid behaviour, wormhole shortcut, Zarzakh default-exclusion, unknown-system and unreachable errors (no live CCP/EVE-Scout calls in CI)
- [x] 8.2 docker-compose service definition (no auth) and README updates for endpoints + `GEODESIC_*` env vars
- [x] 8.3 `cargo clippy --all-targets -- -D warnings` clean; validate the OpenSpec change

# erbridge-geodesic

EVE Online gate-routing REST API. It loads CCP's Static Data Export (SDE),
builds the New Eden gate graph in memory, and serves routing over HTTP with
per-request wormhole overlays.

This is a foundation: black ops staging, wormhole ship-size enforcement, and
authentication are deliberately **out of scope** and land as additive changes.
It ships with **no authentication** and is meant to run as a docker-internal
service (e.g. behind the wormhole mapper) — do not expose it publicly.

## Requirements

- Rust 1.96+ (edition 2024). The release image pins `rust:1.96.0`.

## Run

```sh
cargo run
```

The server binds to `0.0.0.0:5001` by default. On first start it downloads the
SDE (a few MB) and builds the graph; subsequent starts load from the disk cache.

```sh
GEODESIC_PORT=9090 cargo run     # override the port
RUST_LOG=debug cargo run         # more verbose logging
```

With [`just`](https://github.com/casey/just):

```sh
just run           # downloads the live SDE on first start
just run-fixture   # run offline against the checked-in SDE fixture
```

### Docker

A published image is available from GHCR:

```sh
docker pull ghcr.io/erbridge-foundation/geodesic:latest
docker run --rm -p 5001:5001 ghcr.io/erbridge-foundation/geodesic:latest
```

Or build and run locally with compose:

```sh
docker compose up --build
```

The compose service binds to `127.0.0.1:5001` (host-local) and persists the SDE
cache in a named volume. It has no auth — keep it on a trusted network.

## Endpoints

| Method | Path                     | Description                          |
|--------|--------------------------|--------------------------------------|
| POST   | `/route/gate`            | Compute a gate route                 |
| GET    | `/health`                | Build number, graph size, freshness  |
| GET    | `/swagger-ui`            | Interactive API docs                 |
| GET    | `/api-docs/openapi.json` | OpenAPI 3.1 document                 |

### `POST /route/gate`

`from` and `to` accept a system name (case-insensitive) or a numeric SDE id.

```sh
curl -s localhost:5001/route/gate \
  -H 'content-type: application/json' \
  -d '{"from": "Jita", "to": "Amarr", "preference": "shortest"}'
```

```json
{
  "jumps": 11,
  "path": [
    { "system": "Jita", "system_id": 30000142, "security": 0.95, "sec_class": "Highsec", "via": "start" },
    { "system": "Perimeter", "system_id": 30000144, "security": 0.95, "sec_class": "Highsec", "via": "stargate" }
  ]
}
```

Request fields:

| Field             | Type                | Default      | Notes                                                                 |
|-------------------|---------------------|--------------|-----------------------------------------------------------------------|
| `from`, `to`      | name or id          | (required)   | Unknown system → `400`                                                 |
| `preference`      | enum                | `shortest`   | `shortest`, `safest`, `prefer_gates`                                   |
| `avoid`           | array of name/id    | `[]`         | Systems never used as transit hops                                     |
| `use_wormholes`   | bool                | `false`      | When true, `connections[]` are added to the overlay                   |
| `connections`     | array of `{from,to,max_size?}` | `[]` | `max_size` is parsed but **not enforced** (reserved for ship-fit)     |
| `include_thera`   | bool                | `false`      | Add live EVE-Scout Thera signatures to the overlay                    |
| `include_turnur`  | bool                | `false`      | Add live EVE-Scout Turnur signatures to the overlay                   |
| `include_zarzakh` | bool                | `false`      | Allow Zarzakh (30100000) as a transit hop (see below)                |

Responses: `200` with the route, `400` for an unknown system, `404` when no
route exists under the given overlay and preference (`{"error": "unreachable"}`).

Preferences:

- **`shortest`** — minimise hop count; every edge costs 1.
- **`safest`** — strongly prefer highsec; non-highsec hops carry a large finite
  penalty (a route still resolves if no highsec path exists).
- **`prefer_gates`** — apply a small additive penalty per wormhole hop, so a
  wormhole is taken only when it shortens the route by more than that penalty.

Each path step's `via` is `start`, `stargate`, or `wormhole`. A real gate always
labels `stargate` even if a wormhole also connects the same pair.

### Wormhole overlays

User `connections[]` and EVE-Scout Thera/Turnur signatures feed the **same**
per-request overlay — the base graph is never mutated. EVE-Scout signatures are
fetched by a background poller (never on the request path); expired signatures
are dropped when building each request's overlay.

### Zarzakh

Zarzakh's gate mechanic locks a ship to its entry gate for 6 hours, so a transit
route through it strands the traveller. It is excluded from transit by default.
Set `include_zarzakh: true` to opt in — the caller owns the 6-hour-lock warning.

## Configuration

All settings use the `GEODESIC_*` convention and have defaults.

| Variable                              | Default                  | Description                                  |
|---------------------------------------|--------------------------|----------------------------------------------|
| `GEODESIC_PORT`                       | `5001`                   | TCP listen port                              |
| `GEODESIC_CACHE_DIR`                  | platform cache dir       | SDE cache location                           |
| `GEODESIC_SDE_DIR`                    | (unset)                  | Load pre-extracted SDE JSONL from this dir, skipping the download (offline dev/tests); disables the SDE reload poll |
| `GEODESIC_SDE_RELOAD_INTERVAL_SECS`   | `3600`                   | SDE freshness poll; `0` disables             |
| `GEODESIC_EVE_SCOUT_INTERVAL_SECS`    | `600`                    | EVE-Scout poll; `0` disables                 |
| `RUST_LOG`                            | `info`                   | Tracing filter                               |

## SDE cache

The service downloads EVE's Static Data Export on first start and caches the
extracted files on disk. Default locations:

| Platform    | Default path                                                              |
|-------------|--------------------------------------------------------------------------|
| Linux/macOS | `$XDG_CACHE_HOME/erbridge-geodesic/sde/` or `~/.cache/erbridge-geodesic/sde/` |
| Windows     | `%LOCALAPPDATA%\erbridge-geodesic\sde\`                                   |

Override with `GEODESIC_CACHE_DIR`.

> ⚠️ The service **mutates** its cache dir: it writes new build subdirectories
> and prunes old ones. Don't point `GEODESIC_CACHE_DIR` at `tests/fixtures/` or
> any directory whose contents you care about.

The graph hot-reloads in the background: when CCP publishes a newer build, a new
graph is built fully in memory and atomically swapped in. Reload failures are
non-fatal (the current graph keeps serving); only the initial load is fatal.

## Architecture

The full New Eden gate graph (~2.4 MB: systems, undirected gate graph, id/name
lookups, kd-tree) is held behind an `ArcSwap` so background reloads never block
requests. Code is organised by layer:

```
src/
├── main.rs          entry point: load SDE, spawn pollers, serve
├── lib.rs           router wiring + OpenAPI document
├── config.rs        GEODESIC_* env helpers + tracing init
├── app_state.rs     ArcSwap graph + EVE-Scout snapshot
├── model.rs         core domain types (System, GraphData, SecClass)
├── sde/             SDE cache, manifest poll, JSONL parsing
├── graph.rs         build GraphData (graph + lookups + kd-tree)
├── routing.rs       RouteContext overlay + Dijkstra
├── eve_scout.rs     Thera/Turnur signature poller + snapshot
├── services/        business logic (overlay assembly + routing)
├── handlers/        HTTP boundary: load snapshots, call services, return DTOs
├── dto.rs           request/response DTOs (utoipa schemas)
├── error.rs         AppError + IntoResponse mapping
└── tasks.rs         background pollers (SDE reload, EVE-Scout)
```

## Tests

```sh
cargo test              # unit + integration (offline)
tests/hurl/run-hurl.sh  # HURL HTTP-contract suite (boots the fixture server)
just check              # fmt + clippy + test + hurl (CI parity)
```

Everything runs fully offline against a checked-in SDE fixture
(`tests/fixtures/`) — no live calls to CCP or EVE-Scout.

## License

[GNU AGPL v3](LICENSE).

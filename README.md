# erbridge-palu

[![Build](https://github.com/erbridge-foundation/erbridge-palu/actions/workflows/palu.yml/badge.svg)](https://github.com/erbridge-foundation/erbridge-palu/actions/workflows/palu.yml)

---

EVE Online gate-routing REST API. It loads CCP's Static Data Export (SDE),
builds the New Eden gate graph in memory, and serves routing over HTTP with
per-request connection overlays (player-built bridges + wormholes).

Wormhole ship-size enforcement and authentication are deliberately **out of
scope** and land as additive changes. It ships with **no authentication** and is
meant to run as a docker-internal service (e.g. behind the wormhole mapper) — do
not expose it publicly.

## The name

*Palu* is the Carolinian word for a **master navigator** — the Pacific wayfinders
who crossed thousands of miles of open ocean by reading the stars, swells, and
birds, without instruments. Naming a New Eden routing engine after them is a
small tribute to the original pathfinders.

## Requirements

- Rust 1.96+ (edition 2024). The release image pins `rust:1.96.0`.

## Run

```sh
cargo run
```

The server binds to `0.0.0.0:5001` by default. On startup it checks CCP's
manifest and downloads the SDE if the disk cache is missing or stale, then builds
the graph. When the cache is already the latest build, startup loads straight
from disk; if the manifest can't be reached, it serves the cached build and lets
the background poller catch up.

```sh
PALU_PORT=9090 cargo run     # override the port
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
docker pull ghcr.io/erbridge-foundation/palu:latest
docker run --rm -p 5001:5001 ghcr.io/erbridge-foundation/palu:latest
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
| POST   | `/api/v1/route/system`   | Fan out routes from one source to many destinations |
| POST   | `/api/v1/route/blops`    | Stage a fleet into bridge range of a cyno target |
| POST   | `/api/v1/route/range`    | List every system reachable in one jump from a source |
| GET    | `/health`                | App version, SDE version, graph size, freshness |
| GET    | `/swagger-ui`            | Interactive API docs                 |
| GET    | `/api-docs/openapi.json` | OpenAPI 3.1 document                 |

### `POST /api/v1/route/system`

Fans out routes from one shared source `from` to many destinations `to[]` over
the gate graph plus the per-request connection overlay. `from` and each entry of
`to[]` accept a system name (case-insensitive) or a numeric SDE id. A single
route is just `to: ["Amarr"]`. Results are returned one per destination, in
request order; duplicate destinations are permitted and answered positionally.

```sh
curl -s localhost:5001/api/v1/route/system \
  -H 'content-type: application/json' \
  -d '{"from": "Jita", "to": ["Amarr", "Nonexistent"], "preference": "shortest"}'
```

```json
{
  "from": "Jita",
  "results": [
    {
      "to": "Amarr",
      "jumps": 11,
      "path": [
        { "system": "Jita", "system_id": 30000142, "security": 0.95, "sec_class": "Highsec", "via": "start" },
        { "system": "Perimeter", "system_id": 30000144, "security": 0.95, "sec_class": "Highsec", "via": "stargate" }
      ]
    },
    {
      "to": "Nonexistent",
      "error": "unknown_system",
      "message": "system not found: Nonexistent"
    }
  ]
}
```

(The `Amarr` `path` is truncated to two steps here; the real response lists every
hop.)

The `from` is echoed once. Each `results` entry echoes its `to` exactly as sent,
then is **either** a route (`jumps` + `path`) **or** an in-slot failure (`error`
+ `message`, the same code/message a single-request `4xx` would carry). The
overlay and preference are not echoed back — the client already holds what it
sent.

Request fields:

| Field             | Type                | Default      | Notes                                                                 |
|-------------------|---------------------|--------------|-----------------------------------------------------------------------|
| `from`            | name or id          | (required)   | Shared source. Unknown system → `400`                                 |
| `to`              | array of name/id    | (required)   | Destinations. Non-empty and at most 1000; empty or over-cap → `400`   |
| `preference`      | enum                | `shortest`   | `shortest`, `safest`, `prefer_gates`                                   |
| `avoid`           | array of name/id    | `[]`         | Systems never used as transit hops                                     |
| `use_wormholes`   | bool                | `false`      | When true, `wormhole`-typed `connections[]` are added to the overlay (gates only those entries, **not** EVE-Scout) |
| `use_bridges`     | bool                | `false`      | When true, `bridge`-typed `connections[]` are added to the overlay    |
| `connections`     | array of `{type,from,to,max_size?}` | `[]` | Typed connections, **always** ingested and validated; used per their `type` flag. `type` is `bridge` or `wormhole` (required). `max_size` (wormhole only) is parsed but **not enforced** (reserved for ship-fit) |
| `include_thera`   | bool                | `false`      | Add live EVE-Scout Thera signatures to the overlay (independent of `use_wormholes`) |
| `include_turnur`  | bool                | `false`      | Add live EVE-Scout Turnur signatures to the overlay (independent of `use_wormholes`) |
| `include_zarzakh` | bool                | `false`      | Allow Zarzakh (30100000) as a transit hop (see below)                |

Each `connections[]` entry carries a required `type` — `bridge` (a stable
player-built bridge, e.g. an Ansiblex jump gate) or `wormhole` (a transient
signature). The list is **always** ingested and validated: an unknown system in
any entry is a `400` regardless of the use-flags. The use-flags decide what is
*used*, not what is *accepted* — `use_wormholes` gates the `wormhole`-typed
entries and `use_bridges` the `bridge`-typed ones, independently. So a set of
wormholes and bridges with `use_wormholes: false, use_bridges: true` still routes
over the bridges. A missing or unrecognised `type` is rejected (`422`).

The failure model mirrors the request's structure. A **shared-header** problem —
an unresolvable `from`, an unknown system in `connections[]`/`avoid[]` (validated
even when its use-flag is off), or a `to[]` that is empty or over the 1000 cap —
is a request-level `400`, before any route is computed. A **per-destination** problem (unknown or unreachable) is reported
in that destination's `results` entry as an `error`/`message` pair while the
request still returns `200`, so one bad hub never sinks the routes that resolved.
The per-destination `unreachable` failure carries `{"error": "unreachable"}`.

Preferences:

- **`shortest`** — minimise hop count; every edge costs 1.
- **`safest`** — strongly prefer highsec; non-highsec hops carry a large finite
  penalty (a route still resolves if no highsec path exists).
- **`prefer_gates`** — apply a small additive penalty per wormhole hop, so a
  wormhole is taken only when it shortens the route by more than that penalty.

Each path step's `via` is `start`, `stargate`, `wormhole`, or `bridge`. A real
gate always labels `stargate` even if an overlay connection also links the same
pair; a `bridge` hop labels `bridge` and a wormhole hop (user or EVE-Scout)
labels `wormhole`.

### `POST /api/v1/route/blops`

Black-ops staging: given a fleet location `from` (**A**) and a fixed cyno target
`to` (**B**), find the fewest-gate-jump system **★** that is within bridge range
of B, return the gate route A→★, and describe the bridge leg ★→B. Use it to
answer "where do I move the fleet so it can bridge onto the target?".

The bridge range is derived from the hull and Jump Drive Calibration level: when
`ship` is omitted it defaults to the worst (shortest-range) Black Ops hull, and
`jdc_level` defaults to `5`. The A→★ gate leg honours the same `preference`,
`avoid`, and wormhole-overlay knobs as `/route/system`.

```sh
curl -s localhost:5001/api/v1/route/blops \
  -H 'content-type: application/json' \
  -d '{"from": "B-E3KQ", "to": "Otanuomi", "ship": "Sin", "jdc_level": 5}'
```

```json
{
  "chosen": {
    "gate_path": [
      { "system": "B-E3KQ", "system_id": 30000307, "security": -0.26, "sec_class": "Nullsec", "via": "start" },
      { "system": "5T-KM3", "system_id": 30000299, "security": -0.17, "sec_class": "Nullsec", "via": "stargate" },
      { "system": "0R-F2F", "system_id": 30000290, "security": -0.31, "sec_class": "Nullsec", "via": "stargate" }
    ],
    "gate_jumps": 2,
    "bridge": {
      "from": { "system": "0R-F2F", "system_id": 30000290, "security": -0.31, "sec_class": "Nullsec" },
      "to":   { "system": "Otanuomi", "system_id": 30000192, "security": -1.0, "sec_class": "Nullsec" },
      "jump_ly": 5.81,
      "in_range": true
    }
  },
  "alternates": [
    { "system": { "system": "2DWM-2", "system_id": 30000292, "security": -0.2, "sec_class": "Nullsec" }, "gate_jumps": 3, "ly_to_b": 5.48 }
  ],
  "jdc_level": 5,
  "effective_ly": 8.0,
  "defaulted": false
}
```

(`alternates` is truncated to one entry here; the real response lists every
in-range candidate, ranked by fewest gate jumps then closest to B.)

Request fields (A→★ leg shares `preference`, `avoid`, `use_wormholes`,
`use_bridges`, `connections` (typed), `include_thera`, `include_turnur`,
`include_zarzakh` with `/route/system` above):

| Field        | Type           | Default          | Notes                                                              |
|--------------|----------------|------------------|--------------------------------------------------------------------|
| `from`, `to` | name or id     | (required)       | A (fleet) and B (cyno target). Unknown system → `400`              |
| `ship`       | hull name or id | worst Black Ops hull | The bridging hull. Unknown hull → `400`                       |
| `jdc_level`  | int `1..=5`    | `5`              | Jump Drive Calibration; out-of-range → `400` (not clamped)         |

Response fields:

| Field          | Notes                                                                         |
|----------------|-------------------------------------------------------------------------------|
| `chosen`       | The picked route: `gate_path` (A→★, inclusive), `gate_jumps`, and the `bridge` leg |
| `bridge`       | `from` (★), `to` (B), `jump_ly` (★→B distance, 2 dp), `in_range`               |
| `alternates`   | Next-best in-range staging systems, ranked the same way; empty when ★ is reached in zero jumps |
| `jdc_level`    | The JDC level used (echoes the default when omitted)                           |
| `effective_ly` | The bridge range in light-years used for the radius query                      |
| `defaulted`    | `true` when the worst-Black-Ops-hull default was applied (no `ship` given)     |

When the fleet is already in bridge range, ★ is `from` itself at zero gate jumps
and `alternates` is empty. The cyno target B may **not** be in highsec — a
highsec B yields a distinct `400` (only the bridge destination is restricted; ★
may be highsec). Responses: `200` with the staging route, `400` for an unknown
system/hull, invalid `jdc_level`, or a highsec target, and `404` when no in-range
or gate-reachable staging system exists.

### `POST /api/v1/route/range`

Jump-range reachability: given a source system `from`, a hull `ship`, and a
`jdc_level`, list **every system reachable in a single jump** — the spatial
fan-out, sorted nearest-first, with a summary. Use it to answer "from here, with
this hull and these skills, where can I land?".

This endpoint is **planning-oriented**, so it differs from `/route/blops` on
purpose:

- `ship` and `jdc_level` are **required** — there is no worst-hull or
  default-level fallback (a planning answer must be explicit, not assumed).
- There is **no gate or wormhole overlay** — a jump ignores gates, so `avoid`,
  `connections`, and `include_zarzakh` are not accepted.
- An empty reachable set is a valid **`200`** with `reachable: []`, not a `404`.

The reachable set excludes wormhole (J-space) systems, **highsec** (a cyno cannot
be lit in highsec, so no jump may land there), and the source system itself.

```sh
curl -s localhost:5001/api/v1/route/range \
  -H 'content-type: application/json' \
  -d '{"from": "Otanuomi", "ship": "Sin", "jdc_level": 5}'
```

```json
{
  "source": { "system": "Otanuomi", "system_id": 30000192, "security": -1.0, "sec_class": "Nullsec" },
  "hull": { "name": "Sin", "type_id": 22430, "base_ly": 4.0 },
  "jdc_level": 5,
  "effective_ly": 8.0,
  "summary": {
    "reachable_count": 23,
    "farthest_ly": 7.98,
    "by_sec_class": { "Lowsec": 4, "Nullsec": 19 }
  },
  "reachable": [
    { "system": "0R-F2F", "system_id": 30000290, "security": -0.31, "sec_class": "Nullsec", "jump_ly": 5.81 }
  ]
}
```

(`reachable` is truncated to one entry here; the real response lists every
in-range system, sorted ascending by `jump_ly`. `summary` figures are
illustrative.)

Request fields:

| Field       | Type            | Default    | Notes                                                       |
|-------------|-----------------|------------|-------------------------------------------------------------|
| `from`      | name or id      | (required) | Source system. Unknown system → `400`                       |
| `ship`      | hull name or id | (required) | Jumping hull. Unknown hull → `400`; missing → `422`         |
| `jdc_level` | int `1..=5`     | (required) | Jump Drive Calibration; `0`/out-of-range → `400`; missing → `422` |

Response fields:

| Field          | Notes                                                                            |
|----------------|----------------------------------------------------------------------------------|
| `source`       | The source system the jump originates from                                       |
| `hull`         | The resolved hull: `name`, `type_id`, `base_ly` (range before the JDC bonus)     |
| `jdc_level`    | The JDC level used (echoed from the request)                                      |
| `effective_ly` | The effective jump range in light-years used for the radius query                 |
| `summary`      | `reachable_count`, `farthest_ly` (2 dp), and `by_sec_class` (count per class)     |
| `reachable`    | Reachable systems sorted ascending by `jump_ly` (2 dp); never includes highsec or the source |

Responses: `200` with the reachable set (possibly empty), `400` for an unknown
system/hull or a `jdc_level` outside `1..=5`, and `422` when a required field
(`ship`/`jdc_level`) is missing from the body.

### Connection overlays

User `connections[]` (typed `bridge` or `wormhole`) and EVE-Scout Thera/Turnur
signatures feed the **same** per-request overlay — the base graph is never
mutated. Each user entry is always ingested and validated; whether it is *used*
is gated per `type` by `use_bridges` / `use_wormholes`, which are independent of
each other. A `bridge` hop is labelled `via: "bridge"`; a `wormhole` hop
(user-supplied **or** EVE-Scout) is labelled `via: "wormhole"`.

EVE-Scout signatures are their own switches: `include_thera` / `include_turnur`
govern them, **not** `use_wormholes` — so "avoid my own wormhole chain but still
hop through Thera" is `use_wormholes: false, include_thera: true`. A hub's
signatures are also added automatically when it is the route's own `from`/`to`
(Thera is a gateless wormhole and would otherwise be unreachable as an endpoint).
Signatures are fetched by a background poller (never on the request path);
expired signatures are dropped when building each request's overlay.

### Zarzakh

Zarzakh's gate mechanic locks a ship to its entry gate for 6 hours, so a transit
route through it strands the traveller. It is excluded from transit by default.
Set `include_zarzakh: true` to opt in — the caller owns the 6-hour-lock warning.

## Configuration

All settings use the `PALU_*` convention and have defaults.

| Variable                              | Default                  | Description                                  |
|---------------------------------------|--------------------------|----------------------------------------------|
| `PALU_PORT`                       | `5001`                   | TCP listen port                              |
| `PALU_CACHE_DIR`                  | platform cache dir       | SDE cache location                           |
| `PALU_SDE_DIR`                    | (unset)                  | Load pre-extracted SDE JSONL from this dir, skipping the download (offline dev/tests); disables the SDE reload poll |
| `PALU_SDE_RELOAD_INTERVAL_SECS`   | `3600`                   | SDE freshness poll; `0` disables             |
| `PALU_EVE_SCOUT_INTERVAL_SECS`    | `600`                    | EVE-Scout poll; `0` disables                 |
| `RUST_LOG`                            | `info`                   | Tracing filter                               |

## SDE cache

The service downloads EVE's Static Data Export on first start and caches the
extracted files on disk. Default locations:

| Platform    | Default path                                                              |
|-------------|--------------------------------------------------------------------------|
| Linux/macOS | `$XDG_CACHE_HOME/erbridge-palu/sde/` or `~/.cache/erbridge-palu/sde/` |
| Windows     | `%LOCALAPPDATA%\erbridge-palu\sde\`                                   |

Override with `PALU_CACHE_DIR`.

> ⚠️ The service **mutates** its cache dir: it writes new build subdirectories
> and prunes old ones. Don't point `PALU_CACHE_DIR` at `tests/fixtures/` or
> any directory whose contents you care about.

Startup ensures the cache is the latest build before serving (see [Run](#run)).
Thereafter the graph hot-reloads in the background: when CCP publishes a newer
build, a new graph is built fully in memory and atomically swapped in. Reload
failures are non-fatal (the current graph keeps serving); only the initial load
is fatal.

## Architecture

The full New Eden gate graph (~2.4 MB: systems, undirected gate graph, id/name
lookups, kd-tree) is held behind an `ArcSwap` so background reloads never block
requests. Code is organised by layer:

```
src/
├── main.rs          entry point: load SDE, spawn pollers, serve
├── lib.rs           router wiring + OpenAPI document
├── config.rs        PALU_* env helpers + tracing init
├── app_state.rs     ArcSwap graph + EVE-Scout snapshot
├── model.rs         core domain types (System, GraphData, SecClass)
├── sde/             SDE cache, manifest poll, JSONL parsing
├── graph.rs         build GraphData (graph + lookups + kd-tree)
├── routing.rs       RouteContext overlay + Dijkstra
├── range.rs         jump-range math (effective LY from hull + JDC)
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

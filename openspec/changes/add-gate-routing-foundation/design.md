# Design — Gate Routing Foundation

## Context

Greenfield rewrite. A prior iteration (`zz-ref/older-iteration/`) implemented the
full service; this foundation re-establishes the core (SDE → graph → gate routing)
with a cleaner overlay model and no auth, deliberately leaving black ops, JDC, and
ship-fit filtering for additive future changes.

## Shared state & hot-reload

The whole New Eden gate graph is held in memory behind `ArcSwap<GraphData>`.

```
ArcSwap<GraphData>  ◄── handlers .load() a snapshot per request (lock-free)
       ▲
       │ .store(new_graph)
background SDE task: poll manifest build number; if newer →
  download → extract → parse → build GraphData (incl. kd-tree) IN MEMORY →
  only then store(new). Disk cache updated as bookkeeping for next restart.
```

Rules:
- Build the new `GraphData` fully (and validate it parses) **before** the swap.
  Never store a half-built graph; never prune the old build's files until the new
  one is fully extracted.
- Reload failures are logged, non-fatal — keep serving the current `Arc`. Only the
  **initial** load is fatal.
- Memory transiently doubles during a swap (old serving + new building). That is
  the only reason RSS briefly rises on reload — note it in code.

A **second** `ArcSwap<EveScoutSnapshot>` holds Thera/Turnur signatures, refreshed
by its own background poller. Same pattern, independent lifecycle.

## Memory footprint (measured from real SDE)

SDE today: 8,490 systems, 13,970 directed stargates (~6,985 undirected). All
systems carry an `xyz` position.

| Component | Size |
|-----------|------|
| `System` structs × 8,490 (id, name, security, sec_class, coords, region, constellation, is_wormhole) | ~0.85 MB |
| petgraph gate graph (~7k undirected edges, both dirs) | ~0.6 MB |
| id_to_idx + name_to_idx (FxHashMap) | ~0.4 MB |
| kd-tree spatial index (~5,485 K-space points, 28 B + tree overhead) | ~0.3–0.5 MB |
| **Live `GraphData`** | **~2.4 MB** |
| Peak during hot-reload (two graphs) | ~5 MB |
| EVE-Scout snapshot (~tens–hundreds of sigs) | <0.1 MB |

Process RSS comfortably <50 MB (mostly tokio/axum/tls runtime). "Hold it all in
memory" is correct: a Dijkstra over a 2 MB graph stays in cache → 1–3 ms routes.
Raw JSONL is ~8 MB on disk; we parse-and-discard the fields we don't route on
(planets, 8-language names, position2D, radius), keeping ~2 MB live.

## Per-request overlay — one unified mechanism

`RouteContext<'a>` wraps `&GraphData` and expresses every per-request modifier as
either a subtraction (avoid set) or an addition (wormhole edges):

```
SUBTRACT (avoid set):   user avoid[]  ∪  Zarzakh (30100000) unless include_zarzakh
ADD (wormhole edges):   user connections[]
                        ∪ Thera sigs   if include_thera   } from EVE-Scout
                        ∪ Turnur sigs  if include_turnur  }  snapshot
WEIGHT (preference):    edge cost applied during traversal (below)
```

User wormholes and EVE-Scout signatures feed the **same** `add_connection`
overlay — they differ only in source. The base `Arc<GraphData>` is never mutated.

### Edge weights

Cost is composable rather than a single switch:
`cost(edge → dest) = 1 + security_penalty(pref, dest.sec_class) + wh_penalty(pref, edge)`

| Preference | security_penalty | wh_penalty (per WH hop) |
|------------|------------------|--------------------------|
| `shortest` | 0 | 0 |
| `safest` | 0 if highsec, large (10000) otherwise | 0 |
| `prefer_gates` | 0 | small additive constant (default ~2) |

`prefer_gates` uses an **additive** penalty (not a near-infinite wall): a wormhole
is taken only when it saves more jumps than the penalty — i.e. "a WH must shorten
the route by ≥ N to be worth it." Large penalties (not `INFINITY`) keep a route
resolvable even when no fully-preferred path exists.

## EVE-Scout integration

`GET https://api.eve-scout.com/v2/public/signatures` returns an array of signature
objects. Relevant fields per entry:

| Field | Use |
|-------|-----|
| `out_system_id` | 31000005 = Thera, 30002086 = Turnur → partitions the two filters |
| `in_system_id` / `in_system_name` | the K-space (or WH) endpoint |
| `max_ship_size` | `medium`/`large`/`xlarge`/`capital` → reserved for future ship-fit |
| `signature_type` | filter to `"wormhole"` |
| `expires_at` / `remaining_hours` | drop expired sigs at read time |

Background poller default interval ~600 s. Snapshot served read-only; expired sigs
filtered when building each request's overlay.

## Zarzakh

Zarzakh (30100000) gate mechanics lock a ship to its entry gate for 6 hours, so any
*transit* route through it strands the traveller. Default: add it to the avoid set
(never a transit hop; honest "no path" if it would be the only route). The
`include_zarzakh` flag (default `false`) lets the caller opt in and own the 6-hour
warning in the frontend. We do **not** model the lock literally — that is stateful
and out of altitude for a stateless router.

## Forward-compatibility (additive future changes)

Reserved now so later changes touch zero existing request shapes:
- **Wormhole `max_size`** (optional per connection / present on EVE-Scout sigs):
  parsed and stored, **not enforced**. The future ship-fit change adds an optional
  top-level `ship_size` and filters `usable = ship_size <= wh.max_size`. Absent
  `max_size` ⇒ "fits anything"; absent `ship_size` ⇒ "no fit filtering."
- **kd-tree** built now, unqueried. The future black ops change only adds query +
  filter logic.

### Future black ops note (capture, do NOT implement here)

Recorded so the deferred change does not re-derive it wrong: black ops / jump
freighters can jump/**bridge OUT of** highsec but **never INTO** highsec (no cyno
can be lit in highsec). So the future blops feature has two opposite sec-class
filters — permissive on the **staging/origin** side (highsec allowed, gated by a
tactical `allow_highsec_staging` default), strict on the **jump-target** side
(highsec forbidden; low/null/J only). Flattening these into one "exclude highsec"
rule would be wrong. JDC skill adds +20% range/level (`effective = base × (1 +
0.20 × jdc)`, default level 5, echoed in the response). The kd-tree indexes
K-space systems (highsec included, as valid staging origins).

## OpenAPI / Swagger

`utoipa` + `utoipa-axum` `OpenApiRouter` collects handler annotations
automatically (no manual `paths(...)` registry to drift). DTOs derive `ToSchema` /
`IntoParams`. Swagger UI and the OpenAPI JSON are served **unconditionally** —
acceptable because the service is docker-internal with no public exposure.

## Env vars (`GEODESIC_*` convention)

- `GEODESIC_PORT` (default 5001)
- `GEODESIC_CACHE_DIR` (override platform cache dir)
- `GEODESIC_SDE_RELOAD_INTERVAL_SECS` (default 3600; 0 disables)
- `GEODESIC_EVE_SCOUT_INTERVAL_SECS` (default 600; 0 disables)
- `RUST_LOG` (default info)

## Out of scope

Black ops staging, JDC/jump-range math, wormhole ship-size **enforcement**,
authentication, rate limiting, per-key limits, and literal Zarzakh lock modelling.

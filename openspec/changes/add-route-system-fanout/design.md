## Context

`POST /api/v1/route/system` is consumed by a wormhole mapper that fans one
selected system out to several hubs. Partitioning the current single-route request
by what varies reveals the shape: **every field except `to` is shared** across a
pilot's fan-out — `from`, `preference`, `avoid`, `use_wormholes`, `connections`,
and the `include_*` flags are all "one pilot, one map state, one routing policy".
Only the destination varies. So the natural request is a shared header plus a `to`
list, and the natural response is `{ from, results: [...] }`. The compute core
(`compute_gate_route`) already takes one source + one destination against borrowed
snapshots, so this is a handler/DTO reshape, not an algorithm change.

This change supersedes an earlier abandoned approach (a per-element batch: an array
of full route requests). That shape repeated the 8 shared fields — including a
~29-edge wormhole chain — once per destination. Recognising that the workload is
"one pilot / one chain / many destinations" is what motivated the fan-out.

## Decisions

### Replace the endpoint (breaking, pre-prod)

`/api/v1/route/system` becomes the fan-out; a single route is `to: ["X"]`.
Alternatives rejected:

- **Add a separate `/fanout` endpoint** — keeps a clean single-route primitive,
  but the fan-out subsumes the single case (a one-element `to`), so two endpoints
  is redundant surface for a pre-prod service.
- **Accept one-or-many on `to`** — backward compatible, but makes `to`
  (string|array) and the response (object|header+list) polymorphic, which is
  awkward to type on the client. Rejected for the same reason the batch rejected a
  polymorphic body.

Taken now because nothing is in production, so the migration cost is zero.

### Shared header + `{ from, results }` response

The request mirrors the query's structure: a shared header stated once, then the
`to` list. The response mirrors it back: `from` echoed once at the top level, then
a `results` list with one entry per destination (in request order), each echoing
its `to`. The overlay/preference are **not** echoed back — `from` is the only join
key the client needs, and returning the 29-edge chain it just sent would bloat the
response.

### Per-destination result entry, echoing `to` as-sent

Each entry echoes the `to` it answered, **exactly as the client sent it** (raw
`SystemRef` — name or id, original casing), then either the route fields or an
`error`/`message` pair. Echoing as-sent keeps success and failure symmetric (a
failed destination has no resolved system to echo) and lets the client match
entries against its own destination list. The result key is `results` (not
`routes`) because some entries are errors, not routes.

The entry's `error`/`message` MUST match what a single-request `4xx` would emit for
the same `AppError`. A small `pub(crate)` accessor on `AppError` (reusing the
existing `parts()` logic) gives the entry and the `IntoResponse` impl one source of
truth so they cannot drift.

### Failure tiers mirror the request structure

This is the design's central elegance: the request's structure and its failure
model agree.

| Tier | Examples | Result |
|------|----------|--------|
| Shared / header | unresolvable `from`; unknown system in `connections[]`/`avoid[]`; bad `preference`; empty `to[]`; `to` over the sanity cap | request-level `400`; no routes computed |
| Per-destination | unknown destination; unreachable destination | `200`; error carried in that destination's `results` entry |

Shared state is resolved **once**; a failure there cannot be attributed to a single
destination, so it is a whole-request `400`. A destination failure is local to its
slot. (Contrast the abandoned batch, where everything was per-element because
nothing was shared.)

### `to[]` bounds: non-empty, sanity cap 1000

`to` must be non-empty — "route me to nothing" is a malformed query (`400`). The
upper bound is a **sanity cap of 1000**, not a workload limit: any legitimate
fan-out is far smaller, and the cap exists only to stop a runaway/accidental single
request (e.g. `to: [millions]`) from wedging a worker.

Abuse/DoS is explicitly **not** the cap's job and **not** in scope here. The
service is currently docker-internal with no public exposure; if it is ever exposed
publicly it gains **API-key auth + tower rate limiting at the edge**, which own the
abuse boundary. The per-request sanity cap stays useful even then, because rate
limiting bounds request *frequency*, not the cost of a *single* request. See the
project's public-exposure defense plan.

### Resolve shared state once

The handler resolves `from`, the avoid set, and the overlay (user connections +
EVE-Scout snapshots) **once**, then loops the destinations through the existing
`compute_gate_route` core against that prepared state. The graph and EVE-Scout
snapshots are loaded once per request, as today. This is where the fan-out's
efficiency comes from: shared work is done once, not once per destination.

## Load testing (local only, not CI)

A `just load-test` recipe drives [`oha`](https://github.com/hatoo/oha) against the
fixture-booted server (boot/wait/teardown scaffolding shared with the HURL runner:
server backgrounded, wait on `/health`, teardown on exit). `oha` is Rust-native, a
single binary, and drives a POST with a body file and headers via flags.

The fan-out shape makes the load body **smaller and more faithful** than the
abandoned batch's would have been: one source, the verified ~29-edge wormhole chain
stated **once** in the shared header, and a diverse destination list whose routes
span ~1..=39 jumps (a mix of in-chain hops, cross-chain legs, WH→hub routes, and
`safest` variants). The same body is shared with the many-destination HURL test.

- **`oha` is an opt-in dev tool** (`cargo install oha`), not a `Cargo.toml`
  dependency. The recipe hints at installation if missing.
- **Two recipes.** `just load-test` runs `oha` in the **foreground** with the
  terminal attached so its live TUI renders (only the *server* is backgrounded; the
  recipe must not pipe `oha`'s stdout). A `…-plain` sibling passes `--no-tui` for a
  capturable summary.
- **Never in CI.** Load numbers are machine-dependent; `check`
  (`fmt clippy test hurl`) is untouched.

### Hot-cache caveat

Neither Rust nor axum caches responses — every request re-runs the routing, and the
only cache in the service is the on-disk SDE file cache loaded into the graph at
startup. The remaining skew is hardware: replaying the same body keeps the CPU data
cache and branch predictor warm over the same graph nodes, inflating throughput
versus a real many-pilots workload. The diverse destination list (each route
touches a different swath of the graph) mitigates this, so the numbers are a
documented **hot-graph upper bound**, not a production estimate.

## Risks / Trade-offs

- **Untagged result-entry schema in `utoipa`** — the success/error entry is a
  flattened untagged outcome with an echoed `to`; utoipa renders this as a `oneOf`
  (verified on the abandoned batch). A concrete type alias works around the path
  macro's inability to parse a nested generic in `body =`.
- **Breaking change** — accepted because pre-prod; no production migration.
- **Routing-specific shape** — the fan-out is not a reusable batch primitive (it
  bakes in "one source"), so blops/range are not generalised here. That is a
  deliberate trade of generality for fit to the actual workload.

# gate-routing Specification

## Purpose

Compute routes between EVE Online solar systems over the stargate graph plus a
per-request overlay, exposing a routing endpoint with configurable preferences
and an avoid list.

## Requirements

### Requirement: System route endpoint

The service SHALL expose `POST /api/v1/route/system` that computes routes from a
single source to one or more destinations over the gate graph plus a per-request
overlay. The request SHALL be a **fan-out**: a shared header that applies to every
destination, plus a `to` list of destinations.

The shared header SHALL accept `from` (system name or id), a `preference`, an
optional `avoid[]`, an optional `connections[]` list of typed connections with
independent `use_wormholes` / `use_bridges` switches, and optional `include_thera`
/ `include_turnur` / `include_zarzakh` flags. The `to` field SHALL be a non-empty
list of destination system references, bounded by a sanity cap of 1000 entries;
the source, overlay, and routing policy are resolved once and applied to every
destination.

The response SHALL be an object that echoes `from` once and carries a `results`
list with **one entry per destination, in request order**. Each entry SHALL echo
the destination `to` it answered, exactly as supplied in the request. A successful
entry SHALL additionally carry the jump count and an ordered path where each step
identifies the system and whether it was reached via `stargate`, `wormhole`,
`bridge`, or `start`. A failed entry SHALL instead carry an error code and
message identifying the failure (an unresolved or unreachable destination).

The failure model SHALL follow the request's structure. A problem in the shared
header — an unresolvable `from`, an unknown system in `connections[]` or `avoid[]`,
a connection with a missing or unrecognised `type`, or an out-of-range parameter —
SHALL fail the whole request with a client error, because that state is shared and
cannot be attributed to a single destination. An empty `to` list SHALL be rejected
with a client error, and a `to` list exceeding the sanity cap SHALL be rejected
with a client error, in both cases before any route is computed. A failure of an
individual destination SHALL NOT fail the request: provided the header is valid and
`to` is within bounds, the service SHALL return a success status and report
destination-level failures within their corresponding `results` entries. Duplicate
destinations SHALL be permitted and answered positionally.

#### Scenario: Multi-destination fan-out

- **WHEN** a valid header and a `to` list of resolvable destinations are supplied
- **THEN** the response echoes `from` and contains one `results` entry per
  destination, in request order, each echoing its `to` with the jump count and
  ordered path

#### Scenario: Single destination

- **WHEN** `to` contains exactly one destination
- **THEN** the response contains a single `results` entry for it

#### Scenario: Route step over a bridge is labelled bridge

- **WHEN** a route traverses a `bridge` connection supplied with `use_bridges`
- **THEN** the corresponding path step reports `via` of `bridge`

#### Scenario: Per-destination failure does not fail the request

- **WHEN** a `to` list mixes resolvable destinations with one that is unknown or
  unreachable
- **THEN** the request returns a success status, the failing entry carries an error
  code and message (still echoing its `to`), and the other entries carry their
  routes

#### Scenario: Unresolvable shared source

- **WHEN** the shared `from` cannot be resolved to a system
- **THEN** the service returns a request-level client error and computes no routes,
  rather than reporting the failure per destination

#### Scenario: Empty destination list

- **WHEN** `to` is empty
- **THEN** the service returns a request-level client error

#### Scenario: Destination list exceeds the sanity cap

- **WHEN** `to` contains more than 1000 destinations
- **THEN** the service returns a request-level client error and computes no routes

### Requirement: Routing preferences

The service SHALL support three preferences. `shortest` SHALL minimise hop count
(every edge costs 1). `safest` SHALL strongly prefer highsec by penalising
non-highsec destinations. `prefer_gates` SHALL apply a small additive penalty to
each wormhole hop so a wormhole is chosen only when it shortens the route by more
than that penalty. Penalties SHALL be large finite values, never infinity, so a
route still resolves when no fully-preferred path exists.

#### Scenario: safest avoids low/null when a highsec path exists

- **WHEN** preference is `safest` and both a highsec and a shorter low/null path exist
- **THEN** the highsec path is returned

#### Scenario: prefer_gates takes a wormhole only when it shortens enough

- **WHEN** preference is `prefer_gates` and a wormhole shortcut saves fewer jumps
  than the wormhole penalty
- **THEN** the all-gate path is returned instead of the wormhole shortcut

#### Scenario: prefer_gates uses a wormhole that saves enough jumps

- **WHEN** preference is `prefer_gates` and a wormhole shortcut saves more jumps
  than the penalty
- **THEN** the route uses the wormhole

### Requirement: Avoid list

The service SHALL exclude every system in the request's `avoid[]` from being
traversed. Avoided systems SHALL never appear as transit hops. The exclusion
applies to transit only: a system that is the route's own `from` or `to` SHALL
remain usable as that endpoint even when it is in `avoid[]` or excluded by
default (e.g. Zarzakh), since an endpoint is never a transit hop.

#### Scenario: Avoided system is routed around

- **WHEN** a system on the shortest path is in `avoid[]`
- **THEN** the returned route does not pass through that system, or is unreachable
  if no alternative exists

#### Scenario: Excluded system is usable as an endpoint

- **WHEN** a system excluded from transit (in `avoid[]`, or Zarzakh by default)
  is the route's `from` or `to`
- **THEN** the route resolves with that system as the endpoint and does not treat
  it as unreachable

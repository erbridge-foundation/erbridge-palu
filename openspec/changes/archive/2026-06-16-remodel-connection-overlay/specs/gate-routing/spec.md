## MODIFIED Requirements

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

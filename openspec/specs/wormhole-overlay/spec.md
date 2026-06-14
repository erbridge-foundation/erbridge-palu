# wormhole-overlay Specification

## Purpose

Augment the base gate graph per request with wormhole connections — user-supplied,
EVE-Scout Thera/Turnur signatures — and apply the default Zarzakh transit
exclusion, all without mutating the base graph.

## Requirements

### Requirement: User-supplied wormhole connections

When `use_wormholes` is set, the service SHALL add each entry in `connections[]` as
an undirected wormhole edge in the per-request overlay without mutating the base
graph. Each connection entry SHALL accept an optional `max_size` field that is
parsed and stored but **not enforced** in this foundation (reserved for a future
ship-fit change). Self-loops and duplicate connections SHALL be ignored.

#### Scenario: Connection creates a usable shortcut

- **WHEN** a request supplies a wormhole connection between two systems
- **THEN** routing may traverse that edge, and the path step is labelled `wormhole`

#### Scenario: max_size is accepted but not enforced

- **WHEN** a connection includes `max_size`
- **THEN** the request succeeds and routing ignores `max_size` (no ship-fit filtering)

### Requirement: EVE-Scout Thera and Turnur signatures

The service SHALL fetch wormhole signatures from EVE-Scout
(`GET https://api.eve-scout.com/v2/public/signatures`) in a background task on an
interval (`GEODESIC_EVE_SCOUT_INTERVAL_SECS`, default 600; `0` disables) and hold
the parsed snapshot behind an `ArcSwap`. The service SHALL NOT call EVE-Scout on
the request path. When `include_thera` (origin system 31000005) or
`include_turnur` (origin system 30002086) is set, the corresponding signatures
SHALL be added to the per-request overlay using the same mechanism as user
connections. Signatures whose `expires_at` is in the past SHALL be skipped when
building the overlay. Only entries with `signature_type` `wormhole` SHALL be used.

#### Scenario: include_thera injects Thera connections

- **WHEN** a request sets `include_thera` and the snapshot has live Thera signatures
- **THEN** those wormhole edges are available to routing for that request

#### Scenario: EVE-Scout unavailable

- **WHEN** an EVE-Scout poll fails
- **THEN** the error is logged and the service serves the last successful snapshot
  (or an empty overlay if none has succeeded yet)

#### Scenario: Expired signatures are dropped

- **WHEN** a cached signature's `expires_at` is in the past at request time
- **THEN** that signature is not added to the overlay

### Requirement: Zarzakh transit exclusion

The service SHALL exclude Zarzakh (30100000) from transit by default by adding it
to the avoid set. The request SHALL accept an `include_zarzakh` flag defaulting to
`false`; when `true`, Zarzakh SHALL route as a normal system. The service SHALL NOT
model Zarzakh's gate-lock mechanic; the caller owns that decision when opting in.

#### Scenario: Zarzakh is not a transit hop by default

- **WHEN** a route could otherwise pass through Zarzakh and `include_zarzakh` is unset
- **THEN** the returned route does not transit Zarzakh

#### Scenario: Opting in allows Zarzakh transit

- **WHEN** `include_zarzakh` is `true`
- **THEN** Zarzakh may appear as a transit hop in the route

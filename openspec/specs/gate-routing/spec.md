# gate-routing Specification

## Purpose

Compute routes between EVE Online solar systems over the stargate graph plus a
per-request overlay, exposing a routing endpoint with configurable preferences
and an avoid list.

## Requirements

### Requirement: Gate route endpoint

The service SHALL expose `POST /route/gate` that computes a route between two solar
systems over the gate graph plus the per-request overlay. The request SHALL accept
`from`, `to` (system name or id), a `preference`, an optional `avoid[]`, an
optional `use_wormholes` switch with `connections[]`, optional `include_thera` /
`include_turnur` / `include_zarzakh` flags, and SHALL return the jump count and an
ordered path where each step identifies the system and whether it was reached via
`stargate`, `wormhole`, or `start`.

#### Scenario: Resolvable route

- **WHEN** a valid `from` and `to` are supplied and a path exists
- **THEN** the response contains the jump count and the ordered path of systems

#### Scenario: Unknown system

- **WHEN** `from` or `to` cannot be resolved to a system
- **THEN** the service returns a client error identifying the unresolved input

#### Scenario: No path exists

- **WHEN** no route exists under the given overlay and preference
- **THEN** the service returns an unreachable result rather than an arbitrary path

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
traversed. Avoided systems SHALL never appear as transit hops. If `from` or `to`
is itself avoided, the route SHALL be treated as unreachable.

#### Scenario: Avoided system is routed around

- **WHEN** a system on the shortest path is in `avoid[]`
- **THEN** the returned route does not pass through that system, or is unreachable
  if no alternative exists

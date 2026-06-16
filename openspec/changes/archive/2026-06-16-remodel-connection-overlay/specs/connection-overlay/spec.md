## ADDED Requirements

### Requirement: User-supplied typed connections

The request SHALL accept a `connections[]` list that extends the base gate graph
for that request only, without mutating the base graph. Each entry SHALL carry a
required `type` of `bridge` (a stable player-built bridge, e.g. an Ansiblex jump
gate) or `wormhole`, a `from`, and a `to`. An entry whose `type` is missing or
unrecognised SHALL be rejected with a client error.

The service SHALL validate **every** `connections[]` entry on every request,
regardless of the use-flags: an entry referencing an unknown system SHALL fail the
whole request with a client error (it is shared-header state, not attributable to
one destination). Self-loops and duplicate connections SHALL be ignored.

Whether a validated entry is added to the per-request overlay SHALL be governed by
an independent boolean flag per type: `use_wormholes` (default `false`) SHALL gate
`wormhole`-typed entries, and `use_bridges` (default `false`) SHALL gate
`bridge`-typed entries. A `wormhole` entry SHALL be added as an undirected edge
labelled `wormhole`; a `bridge` entry SHALL be added as an undirected edge
labelled `bridge`.

A `wormhole` entry SHALL accept an optional `max_size` field that is parsed and
stored but **not enforced** (reserved for a future ship-fit change). `max_size`
SHALL NOT apply to `bridge` entries.

#### Scenario: Wormhole connection used when use_wormholes is set

- **WHEN** a request supplies a `wormhole` connection and sets `use_wormholes`
- **THEN** routing may traverse that edge, and the path step is labelled `wormhole`

#### Scenario: Bridge connection used when use_bridges is set

- **WHEN** a request supplies a `bridge` connection and sets `use_bridges`
- **THEN** routing may traverse that edge, and the path step is labelled `bridge`

#### Scenario: Type flags are independent

- **WHEN** a request supplies both wormhole and bridge connections with
  `use_wormholes` unset and `use_bridges` set
- **THEN** only the bridge edges are added to the overlay; the wormhole entries
  are validated but not used

#### Scenario: Connections are validated even when their type flag is off

- **WHEN** a request supplies a connection that references an unknown system while
  the matching use-flag is unset
- **THEN** the service returns a request-level client error rather than silently
  ignoring the entry

#### Scenario: Unknown connection type is rejected

- **WHEN** a `connections[]` entry has a missing or unrecognised `type`
- **THEN** the service returns a request-level client error

#### Scenario: max_size is accepted but not enforced

- **WHEN** a `wormhole` connection includes `max_size`
- **THEN** the request succeeds and routing ignores `max_size` (no ship-fit filtering)

### Requirement: EVE-Scout Thera and Turnur signatures

The service SHALL fetch wormhole signatures from EVE-Scout
(`GET https://api.eve-scout.com/v2/public/signatures`) in a background task on an
interval (`PALU_EVE_SCOUT_INTERVAL_SECS`, default 600; `0` disables) and hold
the parsed snapshot behind an `ArcSwap`. The service SHALL NOT call EVE-Scout on
the request path. When `include_thera` (origin system 31000005) or
`include_turnur` (origin system 30002086) is set, the corresponding signatures
SHALL be added to the per-request overlay using the same mechanism as user
connections. These flags SHALL be independent of `use_wormholes`: EVE-Scout
signatures SHALL be governed only by their own `include_*` flags, so a request MAY
exclude user wormhole connections while still including Thera or Turnur signatures.
A hub's signatures SHALL ALSO be added when that hub is the route's own `from` or
`to`, regardless of the `include_*` flag: the flag governs using the hub as a
mid-route shortcut, but reaching it as an endpoint must not require opting in
(Thera is a gateless wormhole and would otherwise be unreachable as a source or
destination). Signatures whose `expires_at` is in the past SHALL be skipped when
building the overlay. Only entries with `signature_type` `wormhole` SHALL be used,
and they SHALL be labelled `wormhole`.

#### Scenario: include_thera injects Thera connections

- **WHEN** a request sets `include_thera` and the snapshot has live Thera signatures
- **THEN** those wormhole edges are available to routing for that request

#### Scenario: EVE-Scout independent of use_wormholes

- **WHEN** a request sets `use_wormholes` to false and `include_thera` to true
- **THEN** Thera signatures are still added to the overlay (the wormhole use-flag
  governs only user-supplied `connections[]`, not EVE-Scout signatures)

#### Scenario: Thera/Turnur usable as an endpoint without the flag

- **WHEN** Thera or Turnur is the route's `from` or `to` and the matching
  `include_*` flag is unset
- **THEN** that hub's live signatures are still added so the endpoint is reachable

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

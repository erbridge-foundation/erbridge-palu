# blops-staging Specification

## Purpose

Given a fleet location and a fixed cyno target, find the best black-ops staging
system within bridge range of the target and route the fleet there over the gate
graph, accounting for the directional cyno security rule, hull and JDC range, and
distinct staging failure modes.

## Requirements

### Requirement: Black-ops staging endpoint

The service SHALL expose `POST /api/v1/route/blops` that, given a fleet location
`from` (A), a target `to` (B), an optional bridging hull `ship`, and an optional
`jdc_level`, returns the gate route from A to the best staging system ★ plus the
bridge leg ★→B. `from` and `to` SHALL accept a system by case-insensitive name or
numeric id (`SystemRef`); `ship` SHALL accept a hull by case-insensitive name or
numeric typeID (`ShipRef`). The endpoint SHALL require no authentication and SHALL
be documented in the OpenAPI specification.

#### Scenario: Fleet routed to a viable staging system

- **WHEN** A and B are valid, B is not highsec, and at least one K-space system is
  within effective bridge range of B and gate-reachable from A
- **THEN** the response returns the chosen gate path A→★, the gate jump count, and
  the bridge leg `{ from: ★, to: B, ly, in_range: true }`

#### Scenario: Endpoints accept name or numeric id

- **WHEN** `from`/`to` are given as numeric system ids and `ship` as a numeric
  typeID
- **THEN** they resolve identically to their name forms

### Requirement: Directional security rule for the bridge

The endpoint SHALL apply the directional security rule to the bridge leg ★→B. The
cyno destination B SHALL NOT be highsec (a cyno cannot be lit in highsec) and a
highsec B SHALL be rejected before routing. The staging origin ★ SHALL be
unrestricted across K-space security classes (bridging out of highsec is legal), so
★ MAY be highsec. These two filters SHALL NOT be merged into a single
"exclude highsec" rule.

#### Scenario: Highsec target is rejected

- **WHEN** B is a highsec system
- **THEN** the request is rejected with a distinct error indicating a cyno cannot be
  lit in highsec, and no route is computed

#### Scenario: Highsec staging origin is allowed

- **WHEN** the fewest-jump in-range staging system ★ is a highsec system and B is
  low/null/J-space
- **THEN** ★ is used as the staging system (highsec ★ is not excluded)

### Requirement: Hull and JDC defaulting

The endpoint SHALL resolve an effective bridge range from the hull's base range and
the JDC level. When `ship` is omitted, it SHALL default to the worst (shortest-base)
black-ops hull, taken as the catalog minimum over the Black Ops group rather than a
hardcoded value. When `jdc_level` is omitted, it SHALL default to 5. A `jdc_level`
outside 0..5 SHALL be rejected. The response SHALL echo the `jdc_level` and
`effective_ly` used and SHALL indicate whether the worst-hull default was applied.

#### Scenario: Worst-hull default when no ship given

- **WHEN** the request omits `ship`
- **THEN** the effective range is computed from the catalog's minimum Black Ops base
  range, and the response indicates the default was applied

#### Scenario: JDC defaults to maxed and is echoed

- **WHEN** the request omits `jdc_level`
- **THEN** level 5 is used and the response echoes `jdc_level` and the resulting
  `effective_ly`

#### Scenario: Out-of-range JDC is rejected

- **WHEN** `jdc_level` is greater than 5 (or negative)
- **THEN** the request is rejected with a validation error rather than being clamped

### Requirement: Staging candidate selection and ranking

The endpoint SHALL find candidate staging systems as the K-space systems within the
effective bridge range of B (a spatial radius query), then compute each candidate's
gate distance from A in a single multi-target shortest-path pass over the gate
graph, honouring the request's routing preference, avoid list, and wormhole overlay
for the A→★ route. Candidates SHALL be ranked by fewest gate jumps first, then by
closest light-year distance to B. The best-ranked candidate SHALL be the chosen ★.

#### Scenario: Fewest gate jumps wins

- **WHEN** two in-range staging systems differ in gate distance from A
- **THEN** the one reachable in fewer gate jumps is chosen

#### Scenario: Ties broken by closest to the target

- **WHEN** two in-range staging systems are an equal number of gate jumps from A
- **THEN** the one closer to B in light-years is chosen

#### Scenario: Routing knobs apply to the gate leg

- **WHEN** the request sets a preference, avoid list, or wormhole connections
- **THEN** they affect the A→★ gate route exactly as for the system-route endpoint

### Requirement: Staging response with fallback candidates

The response SHALL contain the chosen route (gate path A→★, gate jump count, and the
bridge leg with light-year distance and an in-range flag) plus a list of next-best
fallback candidates (system, gate jumps, light-years to B) so a blocked staging
system does not require a second request.

#### Scenario: Fallbacks are included

- **WHEN** more than one in-range staging system is gate-reachable from A
- **THEN** the response includes the chosen route and up to N ranked fallback
  candidates beyond the chosen one

### Requirement: Distinct staging failure modes

The endpoint SHALL distinguish its failure modes rather than returning one generic
error: a highsec target B (cyno impossible), no staging system within range of B,
and a staging system that is not gate-reachable from A. Unknown systems or an
unknown hull SHALL return the existing unknown-resource validation error.

#### Scenario: No candidate within range

- **WHEN** no K-space system lies within the effective bridge range of B
- **THEN** the response indicates no staging system is within range (distinct from a
  gate-unreachable result)

#### Scenario: Staging system not gate-reachable from A

- **WHEN** in-range staging systems exist but none is gate-reachable from A under
  the request's overlay
- **THEN** the response indicates the fleet cannot reach a staging system by gate

#### Scenario: Unknown hull

- **WHEN** `ship` names a hull or typeID not present in the catalog
- **THEN** the request is rejected with the unknown-resource validation error

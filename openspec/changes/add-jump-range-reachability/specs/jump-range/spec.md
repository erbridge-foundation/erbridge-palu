## ADDED Requirements

### Requirement: Jump-range reachability endpoint

The service SHALL expose `POST /api/v1/route/range` that, given a source system
`from`, a required bridging hull `ship`, and a required `jdc_level`, returns every
K-space system reachable from `from` in a single jump at the hull's effective jump
range. `from` SHALL accept a system by case-insensitive name or numeric id
(`SystemRef`); `ship` SHALL accept a hull by case-insensitive name or numeric typeID
(`ShipRef`). The endpoint SHALL require no authentication and SHALL be documented in
the OpenAPI specification.

The effective jump range SHALL be computed from the hull's catalogued base range and
the JDC level using the shared range math (`base_ly × (1 + bonus_per_level × jdc_level)`),
identical to the math used by the staging endpoint.

#### Scenario: Reachable systems returned for a valid request

- **WHEN** `from` is a valid system, `ship` is a catalogued hull, and `jdc_level` is
  in range
- **THEN** the response returns the source system, the resolved hull, the
  `jdc_level` and `effective_ly` used, and the list of reachable systems

#### Scenario: Source and hull accept name or numeric id

- **WHEN** `from` is given as a numeric system id and `ship` as a numeric typeID
- **THEN** they resolve identically to their case-insensitive name forms

### Requirement: Required hull and JDC level

The endpoint SHALL require both `ship` and `jdc_level` in the request; neither has a
default. A missing `ship`, a missing `jdc_level`, or a hull not present in the catalog
SHALL be rejected with the validation error. The `jdc_level` SHALL be within `1..=5`:
every jump-capable hull requires Jump Drive Calibration level 1 at a minimum, so a
`jdc_level` of `0` SHALL be rejected, as SHALL a level greater than 5. This endpoint is
planning-oriented and SHALL NOT substitute a worst-hull or default-level value when an
input is absent.

#### Scenario: Missing hull is rejected

- **WHEN** the request omits `ship`
- **THEN** the request is rejected with a validation error rather than defaulting to a
  worst hull

#### Scenario: Missing JDC level is rejected

- **WHEN** the request omits `jdc_level`
- **THEN** the request is rejected with a validation error rather than defaulting to 5

#### Scenario: JDC level of 0 is rejected

- **WHEN** `jdc_level` is `0`
- **THEN** the request is rejected with a validation error (every jump-capable hull
  requires JDC 1)

#### Scenario: Out-of-range JDC is rejected

- **WHEN** `jdc_level` is greater than 5
- **THEN** the request is rejected with a validation error rather than being clamped

#### Scenario: Unknown hull is rejected

- **WHEN** `ship` names a hull or typeID not present in the catalog
- **THEN** the request is rejected with the unknown-resource validation error

### Requirement: Reachable set construction

The endpoint SHALL find reachable systems as the K-space systems within the effective
jump range of `from` (a spatial radius query over the index). The set SHALL exclude
J-space systems, highsec systems (a cyno cannot be lit in highsec, so no jump may land
there), and the source system itself. The endpoint SHALL NOT apply any gate or wormhole
overlay (avoid list, wormhole connections, Zarzakh opt-in): a jump does not traverse
gates, so those inputs do not affect reachability and SHALL NOT be accepted as routing
knobs for this endpoint.

#### Scenario: Highsec destinations are excluded

- **WHEN** a system within effective jump range of `from` is highsec
- **THEN** it is absent from the reachable set (a cyno cannot be lit there)

#### Scenario: The source system is excluded

- **WHEN** the reachable set is built
- **THEN** the source system `from` is not listed among the reachable systems

#### Scenario: Reachability ignores the gate graph

- **WHEN** a low/null-sec system lies within effective jump range of `from` but is not
  gate-reachable from `from`
- **THEN** it is still included in the reachable set (jumps do not use gates)

### Requirement: Reachability response with summary

The response SHALL list each reachable system with its name, id, security status,
security class, and its jump distance from `from` in light-years, sorted by ascending
light-year distance. The response SHALL include a summary header containing the count
of reachable systems, the farthest light-year distance, and a breakdown of the count by
security class. When no system is reachable, the endpoint SHALL return HTTP 200 with an
empty reachable list and a zero count — an empty result is a valid planning answer, not
an error.

#### Scenario: Reachable systems are sorted nearest-first with distances

- **WHEN** multiple systems are reachable
- **THEN** the response lists them in ascending order of light-year distance from
  `from`, each with its jump-distance in light-years

#### Scenario: Summary reports count, farthest distance, and class breakdown

- **WHEN** the reachable set is non-empty
- **THEN** the summary reports the total reachable count, the farthest light-year
  distance, and the count broken down by security class

#### Scenario: Empty reachable set returns 200

- **WHEN** no K-space, non-highsec system lies within effective jump range of `from`
- **THEN** the endpoint returns HTTP 200 with an empty reachable list and a summary
  count of zero, not an error status

# hull-catalog Specification

## Purpose

Build and expose an in-memory catalog of jump-capable ship hulls derived from the
EVE Static Data Export, providing per-hull base jump ranges, group-based
worst-hull lookups, and pure jump range math.

## Requirements

### Requirement: Jump-capable hull catalog

The service SHALL build an in-memory hull catalog from the SDE that maps every
**published** ship type carrying a `jumpDriveRange` dogma attribute (attribute
**867**, in light-years) to its base jump range, its `groupID`, and its name. The
catalog SHALL be keyed for lookup by both ASCII-lowercased name and numeric typeID.
Range SHALL be read from attribute **867** only; attribute 868
(`jumpDriveConsumptionAmount`, fuel) SHALL NOT be used as range. The catalog SHALL
be mechanic-agnostic: it stores ranges and groups and contains no
jump/bridge/conduit or feature-specific logic.

#### Scenario: Published jump hull is catalogued with its base range

- **WHEN** the SDE contains a published type with a `jumpDriveRange` (867) value
- **THEN** the catalog contains that hull, resolvable by name (case-insensitive)
  and by typeID, with `base_ly` equal to the 867 value and the hull's `groupID`

#### Scenario: Non-jump and unpublished types are excluded

- **WHEN** a type has no `jumpDriveRange` attribute, or is not published
- **THEN** it is absent from the catalog

#### Scenario: Range comes from attribute 867, not fuel

- **WHEN** a hull has both `jumpDriveRange` (867) and
  `jumpDriveConsumptionAmount` (868)
- **THEN** the catalogued `base_ly` is the 867 value, never the 868 value

### Requirement: Worst-hull range lookups computed from the catalog

The catalog SHALL expose the minimum base range over a given `groupID`, computed at
load time, so callers can derive a conservative default without enumerating hulls.
This value SHALL be recomputed on every SDE reload so it self-heals if CCP changes
hull ranges.

#### Scenario: Minimum base range over a group

- **WHEN** a caller requests the minimum base range for the Black Ops group
  (groupID 898)
- **THEN** the catalog returns the smallest `base_ly` among that group's catalogued
  hulls

#### Scenario: Black Ops hulls are enumerated from the SDE

- **WHEN** the catalog is built from a build containing the published Black Ops
  battleships
- **THEN** all of them (not a hand-maintained subset) are present in group 898, and
  the group's minimum base range falls within a sane band (approximately 4 LY)
  rather than being pinned to an exact constant

### Requirement: Jump range math

The service SHALL provide pure range math that computes a hull's effective jump
range as `effective_ly = base_ly × (1 + bonus_per_level × jdc_level)`, where
`jdc_level` defaults to 5 and `bonus_per_level` is the Jump Drive Calibration
skill's `jumpDriveRangeBonus` (attribute **870**) read from the SDE rather than
hardcoded. Jump Drive Calibration SHALL be the only skill applied to jump range
(per-hull-class skills such as the Jump Freighters skill affect hitpoints and fuel,
not range). The math SHALL also provide conversion between light-years and the
squared-metre distances used by the spatial index, using a single light-year
constant.

#### Scenario: Effective range applies the JDC bonus

- **WHEN** effective range is computed for a 4.0 LY base hull at JDC level 5 with a
  per-level bonus of 20%
- **THEN** the result is 8.0 LY

#### Scenario: JDC level defaults to maxed

- **WHEN** effective range is computed without an explicit JDC level
- **THEN** level 5 is assumed

#### Scenario: Light-year to squared-metre conversion is consistent

- **WHEN** a range in light-years is converted to a squared-metre radius and a
  squared-metre distance is converted back to light-years
- **THEN** the conversions are inverse using the same light-year constant, so a
  point exactly at the range maps to the radius boundary

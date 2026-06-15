## MODIFIED Requirements

### Requirement: Hull and JDC defaulting

The endpoint SHALL resolve an effective bridge range from the hull's base range and
the JDC level. When `ship` is omitted, it SHALL default to the worst (shortest-base)
black-ops hull, taken as the catalog minimum over the Black Ops group rather than a
hardcoded value. When `jdc_level` is omitted, it SHALL default to 5. A `jdc_level`
outside `1..=5` SHALL be rejected: every jump-capable hull requires Jump Drive
Calibration level 1 at a minimum, so a `jdc_level` of `0` SHALL be rejected, as SHALL a
level greater than 5. The response SHALL echo the `jdc_level` and `effective_ly` used
and SHALL indicate whether the worst-hull default was applied.

#### Scenario: Worst-hull default when no ship given

- **WHEN** the request omits `ship`
- **THEN** the effective range is computed from the catalog's minimum Black Ops base
  range, and the response indicates the default was applied

#### Scenario: JDC defaults to maxed and is echoed

- **WHEN** the request omits `jdc_level`
- **THEN** level 5 is used and the response echoes `jdc_level` and the resulting
  `effective_ly`

#### Scenario: Out-of-range JDC is rejected

- **WHEN** `jdc_level` is `0` or is greater than 5
- **THEN** the request is rejected with a validation error rather than being clamped

## REMOVED Requirements

### Requirement: User-supplied wormhole connections

**Reason**: Superseded by the typed-connection model in the new `connection-overlay`
capability. `connections[]` entries are now typed (`bridge` / `wormhole`), always
validated, and gated by independent `use_wormholes` / `use_bridges` flags rather
than the single wormhole-only switch.

**Migration**: See `connection-overlay` → "User-supplied typed connections". Add a
`type` to each connection entry; set `use_bridges` to opt into bridge edges.

### Requirement: EVE-Scout Thera and Turnur signatures

**Reason**: Carried forward unchanged (apart from being made explicitly independent
of `use_wormholes`) into the renamed `connection-overlay` capability.

**Migration**: See `connection-overlay` → "EVE-Scout Thera and Turnur signatures".
No request change for existing `include_thera` / `include_turnur` callers.

### Requirement: Zarzakh transit exclusion

**Reason**: Carried forward unchanged into the renamed `connection-overlay`
capability.

**Migration**: See `connection-overlay` → "Zarzakh transit exclusion". No request
change for existing `include_zarzakh` callers.

## Context

The per-request overlay is wormhole-centric. `GateRouteRequest` (and the shared
blops knobs) carry a `use_wormholes: bool` flag and a `connections: Vec<WhConnection>`
list; `resolve_connections` in `src/services/route.rs` short-circuits to an empty
vec when `use_wormholes` is false, so the list is neither validated nor used. An
player-built bridge (e.g. an Ansiblex jump gate) between two systems — is a
legitimate routing shortcut with a different risk profile from a wormhole (stable
and owned vs. transient and collapsing), and callers need to opt into the two
types independently.

This is a pre-1.0 (v0.0.0), docker-internal service with no guaranteed external
consumers, so the API can change without a deprecation window.

## Goals / Non-Goals

**Goals:**

- Type each `connections[]` entry as `bridge` or `wormhole`.
- Always ingest and validate `connections[]`; gate only *use* via two independent
  flags (`use_wormholes`, `use_bridges`), both defaulting `false`.
- Keep EVE-Scout `include_thera` / `include_turnur` independent of `use_wormholes`.
- Label bridge hops `via: "bridge"` in the response.
- Rename the spec `wormhole-overlay` → `connection-overlay`.

**Non-Goals:**

- No ship-size / `max_size` enforcement (still parsed-but-ignored, wormhole-only).
- No new endpoints, no change to the gate graph, EVE-Scout polling, or Zarzakh
  behaviour beyond what the type/flag remodel requires.
- No modelling of bridge fuel, online/offline state, standings, or jump range —
  a supplied bridge connection is taken at face value as a usable edge.
- No persistence of connections; they remain per-request overlay only.

## Decisions

### Hard rename, no backward-compat alias

Replace the old shape outright: `connections[]` entries require `type`, and the
new `use_bridges` flag is added. We will NOT accept the old untyped entry via a
serde default, and we will NOT keep silent-skip behaviour.

*Rationale:* pre-1.0, docker-internal, no external contract to protect. A
compat shim (defaulting missing `type` to `wormhole`) would preserve the exact
behaviour we're trying to correct — silently treating everything as a wormhole —
and carry dead surface forever. A missing `type` now SHALL be a `400`.

*Alternative considered:* `#[serde(default)] type` → `wormhole`. Rejected: hides
the breaking nature and re-introduces the "everything is a wormhole" assumption.

### Tagged enum for connection type, flat on the wire

Add `type` as an internally-tagged discriminator on the connection entry:

```jsonc
{ "type": "bridge", "from": "X", "to": "Y" }
{ "type": "wormhole", "from": "A", "to": "B", "max_size": "xlarge" }
```

Implementation: rename `WhConnection` → `Connection` with a
`#[serde(tag = "type", rename_all = "snake_case")]` enum field, or a flat struct
with a `ConnectionKind` enum field — whichever keeps the utoipa schema clean. The
DTO keeps `from` / `to` shared; `max_size: Option<String>` lives only on the
wormhole variant (or is documented as ignored for bridge). Unknown `type` values
SHALL be rejected by serde (a `400`), not silently dropped.

*Rationale:* one list keeps the "connections extend the map" mental model the user
asked for; the tag makes filtering at use-time trivial and the schema
self-documenting.

### Always validate; gate at overlay-assembly time, not at ingest

`resolve_connections` SHALL resolve every entry's `from`/`to` to node indices on
every request (an unknown system → request-level `400`, consistent with the
shared-header failure model). The use-flags are applied later, when edges are
added to the overlay: a wormhole-typed edge is added only if `use_wormholes`, a
bridge-typed edge only if `use_bridges`.

*Rationale:* matches the user's model ("the list extends the map; the switches
decide what to use") and fixes the current latent bug where a typo in a
`connections[]` system is silently ignored when the wormhole flag is off.

### `via: "bridge"` carried on the overlay edge

The overlay edge already records how it was created so the path step can be
labelled. Add a `bridge` variant to that edge-kind so a hop over a bridge
renders `via: "bridge"`. `RouteStep.via` remains a string on the wire
(`start` / `stargate` / `wormhole` / `bridge`); per the existing
[[routestep-enum-followup]] this stays a `String` for now rather than becoming a
serde enum — that conversion is out of scope here.

### EVE-Scout edges remain wormhole-kind but flag-independent

Thera/Turnur signatures are wormholes and keep `via: "wormhole"`. Their inclusion
stays governed solely by `include_thera` / `include_turnur` (and the
endpoint-reachability carve-out), NOT by `use_wormholes`. So "avoid my wormhole
chain but allow Thera" is `use_wormholes: false, include_thera: true`.

*Rationale:* the user confirmed this is a real route. The `use_wormholes` flag
governs the user-supplied `connections[]` wormholes only.

## Risks / Trade-offs

- **Breaking request shape** → Acceptable pre-1.0; documented as BREAKING in the
  proposal and README. All in-repo callers (HURL, integration, load fixtures) are
  updated in the same change so CI stays green.
- **Naming overlap "use_wormholes" now means two different scopes** (user
  connections vs. EVE-Scout) → Mitigated by spec + README wording making explicit
  that `use_wormholes` gates only `connections[]` wormhole entries; EVE-Scout has
  its own flags. Considered renaming to `use_connection_wormholes` but rejected as
  over-precise for a docker-internal API.
- **`max_size` on a shared struct is meaningless for bridge** → The tagged-enum
  shape keeps it off the bridge variant (or it is documented as ignored), so the
  schema doesn't imply bridge sizing.
- **Spec rename churn** → `wormhole-overlay` archive history is preserved; only the
  live `openspec/specs/` folder is renamed. The change's delta uses the new
  `connection-overlay` name and carries the unchanged EVE-Scout / Zarzakh
  requirements forward so nothing is lost at archive time.

## Migration Plan

1. Land the DTO + service changes and update all in-repo test/load fixtures and
   the README in one commit so `just check` passes.
2. Rename `openspec/specs/wormhole-overlay/` → `openspec/specs/connection-overlay/`
   at archive time (the change's spec delta is authored under the new name).
3. No data migration (overlay is per-request, nothing persisted). Rollback is a
   straight revert; no stored state to unwind.

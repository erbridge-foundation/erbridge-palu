## Why

The per-request overlay only understands wormholes: `connections[]` is gated by a
single `use_wormholes` flag and ignored entirely (not even validated) when that
flag is off. But a connection set can include **player-built bridges** (e.g.
Ansiblex jump gates) — stable owned infrastructure between two systems with a
very different risk profile from a transient wormhole. A caller needs to opt into
bridge shortcuts while excluding wormholes (or the reverse), which the single
flag cannot express.

## What Changes

- **Type each connection.** Every `connections[]` entry gains a required `type`
  field: `bridge` or `wormhole`. (**BREAKING**: existing entries had no `type`.)
- **Always ingest `connections[]`.** The list is parsed and validated on every
  request — unknown systems in a connection now yield a `400` regardless of the
  use-flags. The flags decide what is *used*, not what is *accepted*. (**BREAKING**:
  previously `connections[]` was skipped, and an invalid entry silently ignored,
  when `use_wormholes` was false.)
- **Independent use-flags.** `use_wormholes` (default `false`) toggles only the
  wormhole-typed entries; a new `use_bridges` (default `false`) toggles the
  bridge-typed entries. A set of 5 wormholes + 3 bridges with
  `use_wormholes: false` still routes over the 3 bridges.
- **EVE-Scout stays independent.** `include_thera` / `include_turnur` remain their
  own switches and are **not** governed by `use_wormholes` — avoiding your own
  wormhole chain while still hopping through Thera is a valid route.
- **`max_size` stays wormhole-only.** It remains parsed-but-unenforced on wormhole
  entries and is absent/ignored for bridge entries.
- **New route-step label.** A hop taken over a bridge reports `via: "bridge"`,
  distinct from `wormhole` / `stargate` / `start`.
- **Rename the spec.** `wormhole-overlay` → `connection-overlay`, since it now
  covers bridge and wormhole connections, not just wormholes.

These knobs are shared by `POST /api/v1/route/system` and `POST /api/v1/route/blops`
(the A→★ gate leg), so both endpoints change together.

## Capabilities

### New Capabilities

- `connection-overlay`: The per-request overlay capability, generalised from
  wormhole-only to typed connections (bridge + wormhole) plus EVE-Scout
  signatures and the Zarzakh transit exclusion. This **replaces** the
  `wormhole-overlay` spec (rename + behavioural change), carrying its EVE-Scout
  and Zarzakh requirements forward unchanged.

### Modified Capabilities

- `gate-routing`: The route-step `via` label set gains `bridge` alongside the
  existing `start` / `stargate` / `wormhole`.

## Impact

- **API (BREAKING)**: request bodies for `POST /api/v1/route/system` and
  `POST /api/v1/route/blops` — `connections[]` entries require `type`; new
  `use_bridges` flag; `connections[]` always validated. Response `via` may now be
  `bridge`. Pre-1.0 (v0.0.0), docker-internal, no guaranteed external consumers —
  a hard rename is acceptable; compat handling for the old shape is decided in
  design.
- **Code**: `src/dto.rs` (`WhConnection` → typed connection DTO, request flags),
  `src/services/route.rs` (overlay assembly, `resolve_connections`, `via`
  labelling), `src/services/blops.rs` (shares the overlay knobs), `src/handlers/route.rs`.
- **OpenAPI**: regenerated from the updated DTOs (utoipa).
- **Tests**: `tests/integration.rs`, `tests/hurl/route.hurl`,
  `tests/hurl/wormhole_chain.hurl`, `tests/load/fanout-wh.json`.
- **Docs**: `README.md` overlay/endpoint sections; the renamed openspec spec.
- **Spec tree**: `openspec/specs/wormhole-overlay/` → `openspec/specs/connection-overlay/`.

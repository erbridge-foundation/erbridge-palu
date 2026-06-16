## 1. DTO remodel (`src/dto.rs`)

- [x] 1.1 Replace `WhConnection` with a typed `Connection` DTO: required `type` (`bridge`/`wormhole`) discriminator, shared `from`/`to`, and `max_size: Option<String>` on the wormhole variant only (absent/ignored for bridge). Reject missing/unknown `type` via serde. Keep `ToSchema` derives and `#[schema(example = ...)]`.
- [x] 1.2 Add `use_bridges: bool` (`#[serde(default)]`, default `false`, schema default/example `false`) to `GateRouteRequest` and `BlopsRouteRequest`; update the `use_wormholes` doc-comment to state it gates only wormhole-typed `connections[]` entries (not EVE-Scout).
- [x] 1.3 Update doc-comments on `connections[]` to: "always ingested and validated; entries are used per their `type` flag."
- [x] 1.4 Update `dto.rs` unit tests for the new shape (typed entries, `use_bridges` defaulting false, unknown-type rejection).

## 2. Service overlay logic (`src/services/route.rs`)

- [x] 2.1 Project the new flags into `OverlayInputs` (`use_wormholes`, `use_bridges`, typed `connections`).
- [x] 2.2 Make `resolve_connections` validate **every** entry on every request (resolve `from`/`to` to node indices; unknown system → `AppError` request-level `400`), independent of the use-flags. Return resolved entries tagged with their type.
- [x] 2.3 In overlay assembly, add a wormhole-typed edge only when `use_wormholes`, and a bridge-typed edge only when `use_bridges`. Keep self-loop/duplicate suppression.
- [x] 2.4 Add a `bridge` variant to the overlay edge-kind so its path step renders `via: "bridge"`; wormhole edges (incl. EVE-Scout) keep `via: "wormhole"`.
- [x] 2.5 Confirm EVE-Scout (`include_thera`/`include_turnur`) injection stays gated only by its own flags + the endpoint-reachability carve-out — not by `use_wormholes`.
- [x] 2.6 Update/extend `route.rs` unit tests: bridge used with `use_bridges`; wormhole excluded with `use_wormholes:false` while bridge still routes; connection with unknown system errors even when its flag is off; unknown `type` rejected; `via:"bridge"` on a bridge hop; `use_wormholes:false`+`include_thera:true` still adds Thera.

## 3. Blops + handler wiring

- [x] 3.1 Update `src/services/blops.rs` to pass the new typed connections / `use_bridges` through the shared A→★ overlay (it reuses `OverlayInputs`).
- [x] 3.2 Update `src/handlers/route.rs` if it references `WhConnection`/flags directly; confirm both endpoints compile against the new DTO.
- [x] 3.3 Regenerate/verify the OpenAPI doc: `lib.rs` path-presence tests pass and the new fields/`type` appear in the schema.

## 4. Tests (integration + HURL + load)

- [x] 4.1 Update `tests/integration.rs` for typed connections, `use_bridges`, and a `via:"bridge"` assertion; add the "bridge used while wormholes excluded" and "connection validated when flag off" cases.
- [x] 4.2 Update `tests/hurl/route.hurl` and `tests/hurl/wormhole_chain.hurl` to the new request shape; add a HURL case asserting `via:"bridge"` and one asserting an unknown-`type` → `422` (axum extractor). Kept `wormhole_chain.hurl` name (genuinely wormhole-chain content).
- [x] 4.3 Update `tests/load/fanout-wh.json` (and any other load body) to the typed-connection shape.

## 5. Docs + spec rename

- [x] 5.1 Update `README.md`: the `/route/system` and `/route/blops` request-field tables (`type` on connections, new `use_bridges`, "always validated" note), the "Wormhole overlays" → "Connection overlays" section (covers bridge + wormhole; clarifies EVE-Scout independence), and the `via` value list to include `bridge`.
- [ ] 5.2 At archive time, rename `openspec/specs/wormhole-overlay/` → `openspec/specs/connection-overlay/` (the change's spec delta is authored under the new name; the old name is retired via the REMOVED delta). — handled by `/opsx:archive`.

## 6. Verify

- [x] 6.1 `just check` (fmt + clippy `-D warnings` + test + hurl) passes.
- [x] 6.2 `openspec validate remodel-connection-overlay` passes; run `/opsx:archive` after merge to sync specs and perform the folder rename.

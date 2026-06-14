## Context

The foundation parses two SDE files (`mapSolarSystems`, `mapStargates`) into a
`GraphData` held behind an `ArcSwap`, hot-reloaded from CCP's build manifest. The
kd-tree spatial index over K-space coordinates is built but unqueried, reserved for
jump-range distance queries.

Future jump/bridge features need per-hull jump range. That data lives in the SDE's
`types.jsonl` (every item: name, group, published flag — but **no** dogma) and
`typeDogma.jsonl` (per-type dogma attributes). All facts below were verified by
downloading SDE build 3393779 and cross-checking the in-game client:

- `jumpDriveRange` is dogma attribute **867** (light-years). Attribute **868** is
  `jumpDriveConsumptionAmount` (fuel) — a tempting but wrong source for range.
- The Jump Drive Calibration skill (typeID 21611) carries `jumpDriveRangeBonus`
  (attribute **870**) = 20.0, i.e. +20% range per level. JDC is the **only** skill
  affecting jump range; it is universal across all jump-capable hulls.
- The Jump Freighters skill's per-level bonuses (attributes 1311/1312) are
  hitpoints and fuel, **not** range — confirmed on the Nomad info panel. There is
  no per-hull-class jump-range skill.
- Black Ops is groupID **898** with 6 published hulls, all base 4.0 LY (Sin,
  Redeemer, Widow, Panther, Marshal, Python — the last two are easy to miss when
  hand-listing).

This change imports that data as a shared primitive. It is consumed first by the
follow-up `add-blops-staging` change (the staging endpoint), and later by jump
freighter / titan-bridge / fan-out features. It adds no endpoint of its own.

## Goals / Non-Goals

**Goals:**

- Acquire and parse `types.jsonl` + `typeDogma.jsonl` within the existing SDE
  cache/extract/hot-reload pipeline, with no new operational mechanism.
- Build a mechanic-agnostic `HullCatalog` (name/typeID → base_ly, group) and fold
  it into `GraphData` so it rides the same atomic swap (no build skew).
- Provide pure, SDE-sourced range math (`effective_ly`, LY ↔ squared-metre).
- Ship a trimmed test fixture and a reusable whole-fixture regeneration script.

**Non-Goals:**

- No HTTP endpoint, no auth, no rate limiting.
- No blops/JF/titan/conduit-specific logic — the catalog stores ranges and groups
  only; mechanics live in consuming features.
- No querying of the kd-tree (the staging endpoint does that).
- No modelling of per-hull-class range skills (none exist) or fuel/jump-fatigue.

## Decisions

**Read range from attribute 867; verify nothing from memory.** The single most
load-bearing fact, and the one we got wrong before verifying (the design and a
memory note both said "868"). 868 is fuel. A loose sanity-band test (Black Ops
group min ≈ 4 LY) guards against parsing the wrong attribute, while deliberately
**not** pinning an exact value — the exact SDE value is authoritative, and the
minute differences are the reason to import rather than hardcode.
_Alternative rejected:_ hardcoding hull ranges — drifts silently on rebalance and
defeats the SDE-as-source-of-truth ethos.

**Stream-filter `types.jsonl` during parse; never materialise it.** At ~150 MB it
dwarfs the map files. The parser keeps only published rows that turn out to be
jump-capable, discarding the rest per line (serde already drops unknown fields).
Net live memory is negligible (~tens of hulls); the cost is parse time, not RAM.
_Alternative rejected:_ deserialise-then-filter — would briefly hold 150 MB.

**Catalog rule = "published type with a `jumpDriveRange` (867)".** No hardcoded
groupID allowlist gates membership — the SDE itself defines what can jump. groupID
is recorded as a label (so callers can ask "min over Black Ops 898") but is not a
filter. This self-heals when CCP adds hulls (e.g. Python is a recent Black Ops
addition that a hand-list would miss).
_Alternative rejected:_ narrow the catalog to a fixed group list — reintroduces a
maintained allowlist and breaks named lookups for other jump hulls.

**Fold `HullCatalog` into `GraphData`, not a sibling `ArcSwap`.** Map data and hull
data come from the same SDE build and reload together. One swap guarantees a live
snapshot never mixes systems from build N with hulls from build N−1. The foundation
pinned `GraphData` invariants are about node-index ordering, which a `hulls` field
does not touch, so this is additive.
_Alternative rejected:_ separate `ArcSwap<HullCatalog>` — admits build skew and
adds a second swap to coordinate.

**Range math is a product over a list of bonuses, but ships with exactly one (JDC).**
The verified reality is `base × (1 + 0.20 × jdc)` for every hull — JDC is the only
range skill. We model the formula plainly (one bonus) rather than building a
speculative multi-bonus engine for a per-hull-class skill that does not exist. If
CCP ever introduces one, generalising is a localised change then.
_Alternative rejected:_ a general bonus-list engine now — complexity serving a
case the SDE shows is empty.

**The only hardcoded constant is `LY_IN_METERS = 9.4607e15`.** Everything else —
base range, JDC bonus — is read from the SDE. The light-year is a physical
constant, not a CCP balance lever. Radius queries use squared metres
(`(ly × LY_IN_METERS)²`) to match `kiddo`'s squared-distance API and avoid a
per-candidate `sqrt`.

**Test fixtures: maps full, types trimmed — opposite strategies, same dir.** Map
fixtures are committed whole because the topology *is* the test (real route
counts). Type fixtures have no topology to preserve, so a trimmed subset of the
relevant rows is the right fixture, not a compromise; committing 150 MB would be
bloat. The trimmed type files sit in the same `<build>/` cache fixture dir, loaded
through the normal `GEODESIC_CACHE_DIR` path. Catalog unit tests use tiny inline
JSONL as `sde/parse.rs` already does.

**One regeneration script, filter mirrors the parser.** A committed, reusable
script (fronted by `just update-fixtures`, in the style of `tests/hurl/run-hurl.sh`)
rebuilds the *whole* fixture (maps verbatim + types trimmed) at a single build, so
the two never skew. Its trim predicate is the same rule the catalog parser uses
(published + attribute 867, plus the JDC skill typeID 21611 explicitly), so fixture
and parser cannot drift. The deferred fixture CI chore later just schedules this
script rather than duplicating regeneration logic.

## Risks / Trade-offs

- **Parsing a 150 MB file on cold start / reload adds time.** → Stream-filter so
  memory stays flat; parse time is bounded and happens off the request path
  (background reload task builds the new snapshot before swapping).
- **A future SDE could rename/move the dogma attribute or change its value.** →
  Sanity-band test on the Black Ops group min surfaces gross breakage; the
  fixture-regeneration chore's test failures are *desirable* signals, not noise.
- **Trim filter and parser drifting apart** would silently shrink test coverage. →
  Make the script's filter the same predicate as the parser; document the coupling.
- **`types.jsonl` name collisions / unpublished duplicates** (e.g. an unpublished
  type sharing a name). → Filter on `published` and key by typeID; name lookup is a
  convenience over the published set.

## Migration Plan

Additive and backward-compatible. Adding `types.jsonl`/`typeDogma.jsonl` to the
required `FILES` set means an existing on-disk cache from a prior build is treated
as incomplete and re-fetched on next start — a one-time re-download, not a failure.
The committed test fixture must be regenerated (via the new script) to include the
trimmed type files before tests referencing the catalog pass. No wire-API change
beyond the additive `/health` `hull_count` field; rollback is reverting the change
(the extra cached files are harmless if unused).

## Open Questions

- Exact home for the regeneration script (`scripts/` vs `tests/fixtures/`) — a
  convention choice, settled during implementation; `tests/`-adjacent matches
  `run-hurl.sh`.
- Whether `/health` should also report the catalog's build number separately —
  unnecessary while catalog and graph share one build (the existing `sde_version`
  already covers both).

## Context

The foundation provides gate routing over an `ArcSwap<GraphData>`: a per-request
`RouteContext` overlay (avoid set, wormhole connections, preference) and a
single-target Dijkstra with reusable thread-local scratch. The kd-tree spatial
index over K-space coordinates is built but unqueried, reserved for jump-range
distance queries. The `add-jump-hull-catalog` change adds a `HullCatalog`
(name/typeID → base range, group) and pure range math (`effective_ly`, LY ↔
squared-metre) into the same `GraphData`.

This change consumes those to answer the black-ops hunter scenario. The target B is
*fixed* (a hunter is on it), the clock is running, and the staging system ★ is the
unknown: the fewest-gate-jump K-space system from which the bridging hull can reach
B. The bridge closes the final gap; the fleet drives by gate to ★.

The defining gotcha is the **directional security rule** (the easy-to-get-wrong
one, mis-stated twice during exploration): the rule governs the bridge ★→B, where ★
is the bridge origin and B the cyno destination. A cyno cannot be lit in highsec, so
B must not be highsec; but a fleet *can* bridge out of highsec, so ★ has no security
restriction. Flattening these into one "exclude highsec" filter produces wrong
routes.

## Goals / Non-Goals

**Goals:**

- A `POST /api/v1/route/blops` endpoint returning the gate route to the best
  staging system plus the bridge leg, with inline fallback candidates.
- Correct directional security handling (B not highsec; ★ unrestricted).
- Worst-blops / JDC-5 defaulting, with effective range from the catalog math.
- A **parameterized staging service** reusable by future bridge mechanics, with
  blops specifics confined to the handler.
- Full unit + integration + HURL test coverage.

**Non-Goals:**

- No jump-freighter self-jump routing — that is chained-jump pathfinding, a
  different algorithm, deferred.
- No auth, rate limiting, or fleet-composition / fuel / jump-fatigue modelling.
- No change to existing routing endpoints or the catalog/overlay requirements.

## Decisions

**Single multi-target Dijkstra from A, not one search per candidate.** The radius
query yields a set of in-range candidates; rather than running a full A→candidate
shortest-path for each, run one Dijkstra from A that settles every candidate's gate
distance in a single outward expansion. This is an additive variant of the existing
`shortest_path`: same thread-local scratch, but the stop condition settles all
in-range targets (or exhausts) instead of early-returning at one. The ranked
candidate set then falls out for free, so the inline fallbacks cost nothing extra.
_Alternative rejected:_ N independent single-target searches — N× the work for data
one pass already produces.

**Handler is blops-specific; the staging service is general.** The service signature
takes A, B, an effective range, and security predicates (a "B must satisfy" check
and a "★ may be any of" set) — it does not know the word "blops". The handler owns
blops specifics: the cyno-at-B rule (B not highsec), the worst-blops default set
(catalog min over group 898), and the covert framing. A future titan-bridge endpoint
is then another thin handler over the same service. This mirrors the codebase's
handler→service→graph layering and the foundation's "lay the seam now" ethos.
_Alternative rejected:_ a blops-hardcoded service — would force a rewrite (not a new
handler) when titan bridging arrives.

**Only B gets the highsec guard; ★ is unrestricted.** Encode the directional rule as
two separate predicates, never one. B is checked up front (reject highsec with a
distinct error before any routing). ★ candidates come from the kd-tree, which already
excludes J-space, and are accepted at any K-space security class. The gate route A→★
still honours the user's own `safest`/`avoid` preferences via `RouteContext`, but ★'s
intrinsic security class is not filtered.

**Effective range → squared-metre radius for the kd-tree.** `effective_ly =
base_ly × (1 + 0.20 × jdc_level)` from the catalog math; the radius query uses
`(effective_ly × LY_IN_METERS)²` to match `kiddo`'s squared-distance API and avoid a
per-candidate `sqrt`. Each kept candidate's `ly_to_B` is computed (one `sqrt`) only
for ranking and the response.

**Reuse `SystemRef`; add a mirroring `ShipRef`.** `from`/`to` are `SystemRef`
(name-or-id) unchanged. `ship` is a new `ShipRef` untagged enum with the same
shape, resolving against the catalog's name/typeID lookups. The quoted-id-reads-as-
name behaviour of untagged enums is by design and is not altered (ids are sent as
JSON numbers).

**Distinct failure modes.** B-highsec, no-candidate-in-range, and ★-ungateable are
separate responses, not one generic "unreachable" — they tell the FC different
things (wrong target vs. ship too short-range vs. fleet boxed in). Unknown system /
hull reuse the existing unknown-resource validation error.

## Risks / Trade-offs

- **Multi-target Dijkstra stop condition is subtle** (must settle all in-range
  candidates, not just the first). → Unit-test with multiple candidates at varied
  gate distances; verify the full ranked set, not only the head.
- **Mis-stating the directional rule** (the historical trap). → Dedicated unit tests
  for both halves: highsec B rejected, highsec ★ accepted; assert they are not the
  same predicate.
- **Picking a fixture staging example** that the trimmed hull range + real map make
  reachable. → Choose a known A/B/hull from in-fixture systems and pin the expected
  ★ and gate count, like the existing route tests pin Jita↔Amarr counts.
- **Effective range so large that every system is "in range"** (huge candidate set).
  → The radius query bounds it; in practice blops range (≈8 LY at JDC 5) keeps the
  candidate set small. Optionally cap the returned fallbacks at N.

## Migration Plan

Additive: a new endpoint and a new multi-target search path; no existing route,
schema, or behaviour changes. Ships only after `add-jump-hull-catalog` (the catalog
and range math are prerequisites). Rollback is removing the endpoint; nothing else
depends on it. OpenAPI gains one path; existing clients are unaffected.

## Open Questions

- The fallback count N (fixed small number vs. all-in-range) — default to a small
  cap for a lean hot-path response; settle the exact value in implementation.
- Whether `in_range` can ever be `false` in a returned `chosen` (it should not — a
  candidate is only chosen if within range), so the field mainly documents the
  bridge leg; keep it for clarity and future non-blops reuse.

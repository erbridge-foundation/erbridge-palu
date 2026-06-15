## Context

The codebase already owns every primitive a jump-range fan-out needs:

- `range.rs` — `effective_ly`, `radius_m2`, `ly_between`, `LY_IN_METERS`.
- `GraphData.spatial_index` — a kd-tree over K-space system coordinates (J-space
  excluded by construction), queried via `within_unsorted::<SquaredEuclidean>`.
- `HullCatalog` — `by_name` / `by_type_id` lookups and `bonus_per_level()` (JDC's
  attribute 870, read from the SDE).

The blops staging service (`services/blops.rs`) already runs the exact radius query
this endpoint needs as **step 2** of `compute_staging`, then discards the in-range set
after using it as Dijkstra targets. This change keeps that radius query, drops the gate
routing and overlay entirely, and returns the set directly with a summary.

The endpoint is **planning-oriented** rather than the blops endpoint's fast hunter
query. That stance — the user sits down deliberately, knows their hull and skills, and
wants a trustworthy answer — drives the divergences below.

## Goals / Non-Goals

**Goals:**

- A `POST /api/v1/route/range` endpoint that returns every jump-legal K-space system
  within one jump of a source, given a required hull and JDC level.
- Reuse the existing range math, kd-tree, and catalog with no new catalog surface.
- Fix the blops `jdc_level` validation (`0..=5` → `1..=5`) so the universal "every
  jump-capable hull requires JDC 1" rule is encoded once across both endpoints.

**Non-Goals:**

- No gate/wormhole overlay support (avoid, wormholes, Zarzakh) — jumps ignore gates.
- No worst-hull or default-JDC fallback — inputs are required.
- No mechanic selector (jump vs bridge vs conduit). The three share the same range math
  and the same "destination can't be highsec" rule, so for a reachability fan-out they
  collapse to one set; a selector would be label-only and is omitted.
- No per-hull JDC prerequisite parsing from the SDE. The "JDC 1 minimum" floor is
  universal, so it is a flat `1..=5` validation bound, not derived per hull.

## Decisions

### A new service function, not a generalization of `compute_staging`

The fan-out shares only step 2 (the radius query) with staging; it has no `from`-as-A
gate leg, no Dijkstra, no overlay, no ranking-by-gate-jumps. Folding it into
`compute_staging` would mean threading "skip routing" flags through a function whose
whole shape is about routing. Instead, add a small dedicated fan-out service (e.g.
`services/range.rs::compute_reachable`) that takes a resolved source node, an
`effective_ly`, and the graph, and returns the sorted reachable set plus the summary.
The shared pieces are the already-extracted primitives (`range.rs`, the kd-tree, the
`BlopsSystem`-style system shape and `round2`), not the staging algorithm.

_Alternative considered_: parameterize `compute_staging` with optional routing. Rejected
— it couples two genuinely different queries and complicates the shipped blops path.

### Handler owns validation; service stays mechanic-agnostic

Mirrors the existing layering (`handlers/route.rs` owns blops defaulting/validation;
`services/blops.rs` is generic). The range handler resolves the hull against the catalog,
validates `jdc_level ∈ 1..=5`, computes `effective_ly`, and calls the service. The
service knows only "source + effective range → reachable set", never "jump" vs "bridge".

### `jdc_level` bound is `1..=5`, hardcoded — and the blops bound is corrected to match

Every jump-capable hull requires JDC 1 (same skill, same drive, universal). The SDE
prerequisite data that would *prove* this per hull is not parsed today (the catalog reads
attribute 870 for the bonus, not `requiredSkills`), and parsing it would add deferred
machinery for zero added fidelity since every hull's floor is 1. So the rule is a flat
validation bound. The same mechanic makes blops' current `0..=5` wrong; this change
corrects it to `1..=5` so the rule lives in one place conceptually. Blops is pre-prod, so
this is a safe correction rather than a breaking production change.

_Alternative considered_: derive the minimum JDC from each hull's SDE prerequisites.
Rejected as speculative — no hull differs from the JDC-1 floor today.

### Empty reachable set is `200`, not `404`

For a planning tool, "your hull at this JDC reaches nothing in K-space" is a valid,
actionable answer (move, or train JDC), not a failure. This is a deliberate divergence
from blops' `NoStagingInRange` 404, justified by the planning-vs-fast-query stance, not
an inconsistency to reconcile.

### Destination filter: exclude highsec and the source system

A jump/bridge/conduit destination needs a cyno, which cannot be lit in highsec, so
highsec systems are excluded from the reachable set (same rule as blops' B). J-space is
already excluded by the kd-tree. The source itself sits at 0 ly inside every radius;
listing it is noise, so it is excluded too.

### Response carries a summary header

Beyond the sorted `reachable[]` list (each system with `jump_ly`, rounded via the shared
`round2`), the response includes a summary: `reachable_count`, `farthest_ly`, and a
`by_sec_class` tally. This makes "how big is my range from here" answerable at a glance,
which suits the planning use case. The tally is a simple fold over the filtered set.

### Endpoint path

`POST /api/v1/route/range`, kept under `/route/*` alongside `/route/system` and
`/route/blops` for discoverability and consistent OpenAPI tagging, even though it
produces a set rather than a path.

## Risks / Trade-offs

- **Large response for high-range hulls in dense space** → The set is bounded by K-space
  systems within range; even a maxed JF (10 LY) covers a finite, modest neighbourhood.
  No pagination needed now; the summary header lets callers gauge size. Revisit only if a
  real payload proves unwieldy.
- **Blops behaviour change (`jdc_level:0` now 400s)** → A real semantic shift, but blops
  is not in production and `0` was never a valid jump input. One existing blops
  validation test updates from accepting/clamping to rejecting `0`. Documented as the
  proposal's single BREAKING (pre-prod) item.
- **Hardcoded `1..=5` could drift if CCP changes prerequisites** → Low likelihood; the
  JDC-1 floor has been stable. If a hull ever needs a higher floor, that is the trigger
  to parse SDE prerequisites (the deferred path), not now.
- **Divergent conventions between two routing endpoints** (required inputs, 200-on-empty,
  no overlay) → Intentional and documented as stance-driven, so reviewers do not
  "harmonize" them back toward blops by reflex.

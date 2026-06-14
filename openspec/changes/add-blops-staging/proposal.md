## Why

A black-ops fleet's defining moment: a hunter is cloaked on a target in system B
*right now*, the fleet is in system A, and B is too far to bridge to directly. The
fleet must drive — through gates — until it reaches a system within bridge range of
B, then bridge the final gap. The clock is running, so the answer must be the
fewest-gate route to a viable staging system. Nothing today answers this; the gate
router finds A→B paths, but the staging system is unknown and must be *solved for*
against jump range. This change adds the endpoint that does it.

## What Changes

- Add a new endpoint **`POST /api/v1/route/blops`**. Given a fleet location `A`, a
  fixed target `B`, and an optional bridging hull + JDC level, it returns the gate
  route from `A` to the best staging system `★` (the in-range system reachable in
  the fewest gate jumps) plus the bridge leg `★ → B`, with a few inline fallback
  candidates.
- Consume the **hull catalog + range math** from `add-jump-hull-catalog`: resolve
  the bridging hull to a base range, apply the JDC bonus to get effective
  light-years, and use the kd-tree radius query to find in-range staging
  candidates. When no hull is given, default to the **worst black-ops hull**
  (catalog minimum over the Black Ops group); when no JDC level is given, assume 5.
- Enforce the **directional security rule** correctly: the cyno destination `B`
  must not be highsec (a cyno cannot be lit there) and is rejected up front; the
  staging origin `★` is unrestricted K-space (you can bridge *out of* highsec).
  These two filters are deliberately *not* merged.
- Reuse the existing per-request routing overlay (`RouteContext`: preference,
  avoid, wormhole connections) for the `A → ★` gate route, and add a **multi-target
  Dijkstra** variant that settles every in-range candidate's gate distance in one
  pass (an additive sibling of the existing single-target search).
- Reuse `SystemRef` for `from`/`to` and add a mirroring **`ShipRef`** (name-or-id)
  for the hull. Add the endpoint to the OpenAPI/Swagger documentation.

No breaking changes: a new endpoint and the additive multi-target search; existing
routes, request/response shapes, and behaviour are unchanged.

## Capabilities

### New Capabilities

- `blops-staging`: The `POST /api/v1/route/blops` endpoint — request/response
  contract, the worst-hull/JDC defaulting, the directional security rule (B not
  highsec, ★ unrestricted), the multi-target staging algorithm (kd-tree radius +
  multi-target Dijkstra + ranking by fewest gate jumps then closest light-years),
  the inline-fallback response shape, and its distinct error/unreachable flavours.

### Modified Capabilities

<!-- None. This change reuses gate-routing, wormhole-overlay, and hull-catalog
     machinery without altering their existing requirements. -->

## Impact

- **Depends on** `add-jump-hull-catalog` (the `HullCatalog` and range math must
  exist first).
- **Code**: new handler `src/handlers/route.rs` (or a sibling) for the endpoint;
  a **parameterized staging service** in `src/services/` that takes `A`, `B`,
  effective range, and security predicates — kept blops-agnostic so a future
  titan-bridge endpoint can reuse it (the *handler* owns blops specifics); a
  multi-target Dijkstra variant in `src/routing.rs` (shares the thread-local
  scratch); new DTOs (`ShipRef`, blops request/response) in `src/dto.rs`; a new
  error variant for the highsec-target rejection in `src/error.rs`; OpenAPI
  registration.
- **Tests**: unit (ranking, directional rule, defaulting, multi-target Dijkstra),
  integration (handler→service over the SDE + trimmed-hull fixture), and HURL
  contract tests with a real in-fixture staging example.
- **No** auth, rate limiting, or jump-freighter self-jump routing (a different,
  deferred algorithm). The handler is blops-specific; the service is reusable.

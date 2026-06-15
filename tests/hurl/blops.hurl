# POST /api/v1/route/blops — HTTP contract for the black-ops staging endpoint,
# run against the full real SDE cache fixture (tests/fixtures/sde).
#
# Pinned example: B-E3KQ (A) → Otanuomi (B), both nullsec, bridged by a Sin at
# JDC 5 (8.0 LY). The fleet drives two gate jumps (B-E3KQ → 5T-KM3 → 0R-F2F) to
# the fewest-gate in-range staging system 0R-F2F, then bridges ~5.81 LY to B.

# ── happy path: route to the expected staging system + bridge leg ────────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "B-E3KQ",
  "to": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 5
}

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.effective_ly" == 8.0
jsonpath "$.jdc_level" == 5
jsonpath "$.defaulted" == false
jsonpath "$.chosen.gate_jumps" == 2
jsonpath "$.chosen.gate_path[0].system" == "B-E3KQ"
jsonpath "$.chosen.gate_path[0].via" == "start"
jsonpath "$.chosen.gate_path[-1].system" == "0R-F2F"
jsonpath "$.chosen.bridge.from.system" == "0R-F2F"
jsonpath "$.chosen.bridge.to.system" == "Otanuomi"
jsonpath "$.chosen.bridge.in_range" == true
jsonpath "$.chosen.bridge.jump_ly" == 5.81
jsonpath "$.chosen.bridge.jump_ly" < 8.0
jsonpath "$.alternates" count > 0

# ── defaulting: no ship → worst Black Ops hull, jdc defaults to maxed ─────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "B-E3KQ",
  "to": "Otanuomi"
}

HTTP 200
[Asserts]
jsonpath "$.defaulted" == true
jsonpath "$.jdc_level" == 5
jsonpath "$.chosen.bridge.from.system" == "0R-F2F"

# ── already in range: zero gate jumps, no fallback candidates ────────────────
# Starting from the staging system itself (0R-F2F ~5.81 LY from Otanuomi) the
# fleet is already in bridge range: ★ is 0R-F2F at zero jumps and there are no
# alternates to offer.
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "0R-F2F",
  "to": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 5
}

HTTP 200
[Asserts]
jsonpath "$.chosen.gate_jumps" == 0
jsonpath "$.chosen.gate_path" count == 1
jsonpath "$.chosen.gate_path[0].system" == "0R-F2F"
jsonpath "$.chosen.gate_path[0].via" == "start"
jsonpath "$.chosen.bridge.from.system" == "0R-F2F"
jsonpath "$.chosen.bridge.to.system" == "Otanuomi"
jsonpath "$.chosen.bridge.in_range" == true
jsonpath "$.alternates" count == 0

# ── directional rule: highsec target B is rejected distinctly ────────────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "ship": "Sin"
}

HTTP 400
[Asserts]
jsonpath "$.error" == "cyno_target_highsec"

# ── unknown hull → 400 ───────────────────────────────────────────────────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "B-E3KQ",
  "to": "Otanuomi",
  "ship": "Rifter"
}

HTTP 400
[Asserts]
jsonpath "$.error" == "unknown_hull"

# ── invalid jdc_level (out of 1..=5) → 400, not clamped ──────────────────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "B-E3KQ",
  "to": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 9
}

HTTP 400
[Asserts]
jsonpath "$.error" == "invalid_param"

# ── jdc_level 0 → 400 (every jump-capable hull requires JDC 1) ────────────────
POST {{base_url}}/api/v1/route/blops
Content-Type: application/json
{
  "from": "B-E3KQ",
  "to": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 0
}

HTTP 400
[Asserts]
jsonpath "$.error" == "invalid_param"

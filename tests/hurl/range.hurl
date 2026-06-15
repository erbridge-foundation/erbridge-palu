# POST /api/v1/route/range — HTTP contract for the jump-range reachability
# fan-out, run against the full real SDE cache fixture (tests/fixtures/sde).
#
# Planning-oriented: ship and jdc_level are required (no defaults), there is no
# gate/wormhole overlay, and an empty reachable set is a valid 200.
#
# Pinned example: from Otanuomi (nullsec) a Sin at JDC 5 (8.0 LY) reaches a set
# of nearby K-space systems including 0R-F2F (~5.81 LY — the same pair the blops
# staging test pins). No gates are involved; this is a pure spatial fan-out.

# ── happy path: reachable systems + echoed hull + summary ────────────────────
POST {{base_url}}/api/v1/route/range
Content-Type: application/json
{
  "from": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 5
}

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.effective_ly" == 8.0
jsonpath "$.jdc_level" == 5
jsonpath "$.hull.name" == "Sin"
jsonpath "$.hull.type_id" == 22430
jsonpath "$.hull.base_ly" == 4.0
jsonpath "$.source.system" == "Otanuomi"
jsonpath "$.reachable" count > 0
# The source is never in its own reachable set.
jsonpath "$.reachable[*].system" not contains "Otanuomi"
# 0R-F2F is reachable at the pinned rounded distance.
jsonpath "$.reachable[*].system" contains "0R-F2F"
jsonpath "$.reachable[?(@.system=='0R-F2F')].jump_ly" == 5.81
# No highsec destination appears (a cyno cannot be lit in highsec).
jsonpath "$.reachable[*].sec_class" not contains "Highsec"
jsonpath "$.summary.reachable_count" > 0
jsonpath "$.summary.by_sec_class.Highsec" not exists

# ── required hull: omitting ship is rejected (422 from the JSON extractor) ────
POST {{base_url}}/api/v1/route/range
Content-Type: application/json
{
  "from": "Otanuomi",
  "jdc_level": 5
}

HTTP 422

# ── required jdc_level: omitting it is rejected (422 from the JSON extractor) ──
POST {{base_url}}/api/v1/route/range
Content-Type: application/json
{
  "from": "Otanuomi",
  "ship": "Sin"
}

HTTP 422

# ── jdc_level 0 → 400 (every jump-capable hull requires JDC 1) ────────────────
POST {{base_url}}/api/v1/route/range
Content-Type: application/json
{
  "from": "Otanuomi",
  "ship": "Sin",
  "jdc_level": 0
}

HTTP 400
[Asserts]
jsonpath "$.error" == "invalid_param"

# ── unknown hull → 400 ───────────────────────────────────────────────────────
POST {{base_url}}/api/v1/route/range
Content-Type: application/json
{
  "from": "Otanuomi",
  "ship": "Rifter",
  "jdc_level": 5
}

HTTP 400
[Asserts]
jsonpath "$.error" == "unknown_hull"

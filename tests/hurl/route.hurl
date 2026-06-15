# POST /api/v1/route/system — HTTP contract for the routing endpoint, run
# against the full real SDE cache fixture (tests/fixtures/sde).
#
# The endpoint is a fan-out: a shared header plus `to: [...]`, returning
# `{ from, results: [...] }` with one entry per destination. A single route is
# `to: ["X"]`; these requests assert against `$.results[0]`. A destination
# failure (unknown / unreachable) is reported in-slot at HTTP 200; only a
# shared-header failure (bad `from`/connection, empty/over-cap `to[]`) is a 400.

# ── shortest: Jita → Amarr is 11 gate jumps ──────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr"],
  "preference": "shortest"
}

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.from" == "Jita"
jsonpath "$.results" count == 1
jsonpath "$.results[0].to" == "Amarr"
jsonpath "$.results[0].jumps" == 11
jsonpath "$.results[0].path[0].system" == "Jita"
jsonpath "$.results[0].path[0].via" == "start"
jsonpath "$.results[0].path[-1].system" == "Amarr"

# ── safest: stays highsec, longer than shortest ──────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr"],
  "preference": "safest"
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" > 11
jsonpath "$.results[0].path[*].sec_class" contains "Highsec"
# Every hop on the safest route is highsec.
jsonpath "$.results[0].path[*].sec_class" not contains "Lowsec"
jsonpath "$.results[0].path[*].sec_class" not contains "Nullsec"

# ── wormhole shortcut: Jita ↔ Akhragan collapses the trip to 4 jumps ──────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr"],
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Akhragan" } ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 4
jsonpath "$.results[0].path[1].system" == "Akhragan"
jsonpath "$.results[0].path[1].via" == "wormhole"
jsonpath "$.results[0].path[-1].system" == "Amarr"

# ── prefer_gates: marginal WH is skipped (gate kept, labelled stargate) ───────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Perimeter"],
  "preference": "prefer_gates",
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Perimeter" } ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 1
jsonpath "$.results[0].path[1].via" == "stargate"

# ── connection max_size is accepted but not enforced ─────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr"],
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Akhragan", "max_size": "medium" } ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 4

# ── numeric system ids resolve (Jita 30000142 → Perimeter 30000144) ──────────
# The echoed `from`/`to` come back as the bare numbers the client sent.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": 30000142,
  "to": [30000144]
}

HTTP 200
[Asserts]
jsonpath "$.from" == 30000142
jsonpath "$.results[0].to" == 30000144
jsonpath "$.results[0].jumps" == 1

# ── avoid: Perimeter is not transited ────────────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr"],
  "avoid": ["Perimeter"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].path[*].system" not contains "Perimeter"

# ── Zarzakh opt-in allows a 2-jump transit (G-0Q86 → Zarzakh → H-PA29) ───────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": ["H-PA29"],
  "include_zarzakh": true
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 2
jsonpath "$.results[0].path[*].system" contains "Zarzakh"

# ── Zarzakh excluded by default: the route detours around it (no Zarzakh hop) ──
# On the full SDE a long gate detour exists, so excluding Zarzakh reroutes rather
# than failing — the key contract is that Zarzakh is never a transit hop.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": ["H-PA29"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].path[*].system" not contains "Zarzakh"
jsonpath "$.results[0].jumps" > 2

# ── Zarzakh is usable as the DESTINATION without opt-in (endpoint != transit) ──
# G-0Q86 -> Zarzakh is 1 gate jump; the default exclusion is about transit, so
# routing to Zarzakh must still resolve.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": ["Zarzakh"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 1
jsonpath "$.results[0].path[-1].system" == "Zarzakh"

# ── Zarzakh is usable as the SOURCE without opt-in ───────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Zarzakh",
  "to": ["H-PA29"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 1
jsonpath "$.results[0].path[0].system" == "Zarzakh"

# ── unknown DESTINATION → 200 with in-slot error (echoing its `to`) ──────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Nowhere"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].error" == "unknown_system"
jsonpath "$.results[0].to" == "Nowhere"

# ── unreachable DESTINATION → 200 with in-slot error ─────────────────────────
# A gateless wormhole with no overlay can't be reached; the failure is local to
# the destination's slot, so the request still succeeds.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["J105443"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].error" == "unreachable"
jsonpath "$.results[0].to" == "J105443"

# ── unknown SHARED `from` → request-level 400 (no results computed) ──────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Nowhere",
  "to": ["Jita"]
}

HTTP 400
[Asserts]
jsonpath "$.error" == "unknown_system"

# ── empty `to[]` → request-level 400 ─────────────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": []
}

HTTP 400
[Asserts]
jsonpath "$.error" == "invalid_param"

# ── multi-destination fan-out: one header, several hubs, in request order ─────
# Echoes `from` once; one result per destination, each echoing its `to`.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": ["Amarr", "Perimeter", "Nowhere"]
}

HTTP 200
[Asserts]
jsonpath "$.from" == "Jita"
jsonpath "$.results" count == 3
jsonpath "$.results[0].to" == "Amarr"
jsonpath "$.results[0].jumps" == 11
jsonpath "$.results[1].to" == "Perimeter"
jsonpath "$.results[1].jumps" == 1
# The unknown destination fails in its own slot; the others still resolved.
jsonpath "$.results[2].to" == "Nowhere"
jsonpath "$.results[2].error" == "unknown_system"

# ── over-cap `to[]` (1001 destinations) → request-level 400 ──────────────────
# Body is read from a committed fixture (1001 copies of "Jita").
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
file,tests/hurl/fanout-over-cap.json;

HTTP 400
[Asserts]
jsonpath "$.error" == "invalid_param"

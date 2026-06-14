# POST /api/v1/route/system — HTTP contract for the routing endpoint, run
# against the full real SDE cache fixture (tests/fixtures/sde).

# ── shortest: Jita → Amarr is 11 gate jumps ──────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "preference": "shortest"
}

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.jumps" == 11
jsonpath "$.path[0].system" == "Jita"
jsonpath "$.path[0].via" == "start"
jsonpath "$.path[-1].system" == "Amarr"

# ── safest: stays highsec, longer than shortest ──────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "preference": "safest"
}

HTTP 200
[Asserts]
jsonpath "$.jumps" > 11
jsonpath "$.path[*].sec_class" contains "Highsec"
# Every hop on the safest route is highsec.
jsonpath "$.path[*].sec_class" not contains "Lowsec"
jsonpath "$.path[*].sec_class" not contains "Nullsec"

# ── wormhole shortcut: Jita ↔ Akhragan collapses the trip to 4 jumps ──────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Akhragan" } ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 4
jsonpath "$.path[1].system" == "Akhragan"
jsonpath "$.path[1].via" == "wormhole"
jsonpath "$.path[-1].system" == "Amarr"

# ── prefer_gates: marginal WH is skipped (gate kept, labelled stargate) ───────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Perimeter",
  "preference": "prefer_gates",
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Perimeter" } ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 1
jsonpath "$.path[1].via" == "stargate"

# ── connection max_size is accepted but not enforced ─────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "use_wormholes": true,
  "connections": [ { "from": "Jita", "to": "Akhragan", "max_size": "medium" } ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 4

# ── numeric system ids resolve (Jita 30000142 → Perimeter 30000144) ──────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": 30000142,
  "to": 30000144
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 1

# ── avoid: Perimeter is not transited ────────────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "avoid": ["Perimeter"]
}

HTTP 200
[Asserts]
jsonpath "$.path[*].system" not contains "Perimeter"

# ── Zarzakh opt-in allows a 2-jump transit (G-0Q86 → Zarzakh → H-PA29) ───────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": "H-PA29",
  "include_zarzakh": true
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 2
jsonpath "$.path[*].system" contains "Zarzakh"

# ── Zarzakh excluded by default: the route detours around it (no Zarzakh hop) ──
# On the full SDE a long gate detour exists, so excluding Zarzakh reroutes rather
# than failing — the key contract is that Zarzakh is never a transit hop.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": "H-PA29"
}

HTTP 200
[Asserts]
jsonpath "$.path[*].system" not contains "Zarzakh"
jsonpath "$.jumps" > 2

# ── Zarzakh is usable as the DESTINATION without opt-in (endpoint != transit) ──
# G-0Q86 -> Zarzakh is 1 gate jump; the default exclusion is about transit, so
# routing to Zarzakh must still resolve.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": "Zarzakh"
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 1
jsonpath "$.path[-1].system" == "Zarzakh"

# ── Zarzakh is usable as the SOURCE without opt-in ───────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Zarzakh",
  "to": "H-PA29"
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 1
jsonpath "$.path[0].system" == "Zarzakh"

# ── unknown system → 400 ─────────────────────────────────────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Nowhere",
  "to": "Jita"
}

HTTP 400
[Asserts]
jsonpath "$.error" == "unknown_system"

# ── unreachable: a gateless wormhole with no overlay can't be reached ─────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "Jita",
  "to": "J105443"
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

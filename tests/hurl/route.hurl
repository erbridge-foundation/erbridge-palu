# POST /route/gate — HTTP contract for the routing endpoint, run against the
# SDE fixture (61 systems; Jita↔Amarr corridor, Zarzakh trio, HD-JVQ island).

# ── shortest: Jita → Amarr is 11 gate jumps ──────────────────────────────────
POST {{base_url}}/route/gate
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
POST {{base_url}}/route/gate
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
POST {{base_url}}/route/gate
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
POST {{base_url}}/route/gate
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
POST {{base_url}}/route/gate
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
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": 30000142,
  "to": 30000144
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 1

# ── avoid: Perimeter is not transited ────────────────────────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "Jita",
  "to": "Amarr",
  "avoid": ["Perimeter"]
}

HTTP 200
[Asserts]
jsonpath "$.path[*].system" not contains "Perimeter"

# ── Zarzakh excluded by default → unreachable (404) ──────────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "G-0Q86",
  "to": "H-PA29"
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

# ── Zarzakh opt-in allows transit ────────────────────────────────────────────
POST {{base_url}}/route/gate
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

# ── unknown system → 400 ─────────────────────────────────────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "Nowhere",
  "to": "Jita"
}

HTTP 400
[Asserts]
jsonpath "$.error" == "unknown_system"

# ── unreachable across disconnected components → 404 ─────────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "Jita",
  "to": "HD-JVQ"
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

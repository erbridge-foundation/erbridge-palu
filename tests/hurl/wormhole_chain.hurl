# Wormhole-chain routing over a real, scanned wormhole map.
#
# Two separately-scanned chains, each anchored on a wormhole system with K-space
# exits:
#   chain A — J120455 (exits: Lashkai, Orduin, DDI-B7, Amoderia, HL-VZX)
#   chain B — J172840 (exits: QPTT-F, AU2V-J, Atai, Iosantin, Martha, Adar, O-TVTD)
#
# There is NO wormhole-only path between the two chains — they connect only
# through real K-space gates between their exits (closest: Lashkai <-> Martha,
# 9 gate jumps). Each request supplies the relevant chain links as `connections[]`;
# the router interleaves `wormhole` overlay hops with real `stargate` hops.
#
# Jump counts below were cross-checked against an in-game wormhole routing tool:
#   J120455 <-> J172840          = 14
#   J120455  -> Jita  (shortest) = 13
#   J120455  -> Jita  (safest)   = 25
#
# Run against the SDE cache fixture (full real SDE):
#   GEODESIC_CACHE_DIR=tests/fixtures/sde GEODESIC_SDE_RELOAD_INTERVAL_SECS=0 \
#   GEODESIC_EVE_SCOUT_INTERVAL_SECS=0 cargo run

# ── J120455 -> J172840: 14 jumps, bridging the two chains via K-space ─────────
# J120455 -(WH)-> Lashkai -(9 gates through Amarr)-> Martha -(4 WH)-> J172840.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "J172840",
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"Orduin"},
    {"from":"J120455","to":"DDI-B7"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"Amoderia"}, {"from":"J131142","to":"HL-VZX"},
    {"from":"J131142","to":"J133011"},
    {"from":"J133011","to":"J133525"}, {"from":"J133011","to":"J151319"},
    {"from":"J172840","to":"J213956"}, {"from":"J172840","to":"O-TVTD"},
    {"from":"J213956","to":"J113453"}, {"from":"J213956","to":"J161411"},
    {"from":"J113453","to":"J162132"}, {"from":"J162132","to":"J112241"},
    {"from":"J113453","to":"QPTT-F"}, {"from":"J113453","to":"AU2V-J"},
    {"from":"J161411","to":"J145155"}, {"from":"J161411","to":"J131204"},
    {"from":"J161411","to":"J122832"},
    {"from":"J145155","to":"J131852"}, {"from":"J145155","to":"Atai"},
    {"from":"J145155","to":"J140810"}, {"from":"J145155","to":"Iosantin"},
    {"from":"J145155","to":"J105415"},
    {"from":"J131204","to":"J222104"}, {"from":"J131204","to":"Martha"},
    {"from":"J122832","to":"J113925"}, {"from":"J122832","to":"Adar"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 14
jsonpath "$.path[0].system" == "J120455"
jsonpath "$.path[0].via" == "start"
jsonpath "$.path[-1].system" == "J172840"
jsonpath "$.path[-1].via" == "wormhole"
# The route leaves chain A via a wormhole into K-space (Lashkai)...
jsonpath "$.path[1].system" == "Lashkai"
jsonpath "$.path[1].via" == "wormhole"
# ...crosses real space on stargates...
jsonpath "$.path[*].via" contains "stargate"
# ...and re-enters chain B's wormholes via Martha.
jsonpath "$.path[*].system" contains "Martha"

# ── reverse is symmetric: J172840 -> J120455 is also 14 ───────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J172840",
  "to": "J120455",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"J133011"},
    {"from":"J133011","to":"J133525"}, {"from":"J133011","to":"J151319"},
    {"from":"J172840","to":"J213956"},
    {"from":"J213956","to":"J113453"}, {"from":"J213956","to":"J161411"},
    {"from":"J161411","to":"J131204"}, {"from":"J131204","to":"Martha"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 14

# ── no connections: the two chains are disconnected -> unreachable ────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "J172840"
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

# ── only chain A supplied: can reach its exits, but not J172840 ───────────────
# Without chain B's wormholes there is no way back into J-space on the far side,
# so the route is unreachable even though K-space is traversable.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "J172840",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"Orduin"},
    {"from":"J120455","to":"DDI-B7"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"Amoderia"}, {"from":"J131142","to":"HL-VZX"},
    {"from":"J131142","to":"J133011"},
    {"from":"J133011","to":"J133525"}, {"from":"J133011","to":"J151319"}
  ]
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

# ── a wormhole NOT on the map is unreachable even with the full chain ─────────
# J105443 is a real J-space system with no gates and no overlay edge, so no
# amount of chain connections can reach it.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "J105443",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"J133011"}
  ]
}

HTTP 404
[Asserts]
jsonpath "$.error" == "unreachable"

# ── J120455 -> Jita, shortest = 13 ───────────────────────────────────────────
# Shortest exits deep into chain A: J120455 -(WH)-> J131142 -(WH)-> Amoderia
# (0.4, Verge Vendor), then 11 gates to Jita.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "Jita",
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"Orduin"},
    {"from":"J120455","to":"DDI-B7"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"Amoderia"}, {"from":"J131142","to":"HL-VZX"},
    {"from":"J131142","to":"J133011"},
    {"from":"J133011","to":"J133525"}, {"from":"J133011","to":"J151319"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 13
jsonpath "$.path[0].system" == "J120455"
jsonpath "$.path[1].via" == "wormhole"
jsonpath "$.path[2].system" == "Amoderia"
jsonpath "$.path[-1].system" == "Jita"
jsonpath "$.path[-1].via" == "stargate"

# ── J120455 -> Jita, safest = 25: all-highsec via Orduin ─────────────────────
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J120455",
  "to": "Jita",
  "preference": "safest",
  "use_wormholes": true,
  "connections": [
    {"from":"J120455","to":"Lashkai"}, {"from":"J120455","to":"Orduin"},
    {"from":"J120455","to":"DDI-B7"}, {"from":"J120455","to":"J131142"},
    {"from":"J131142","to":"Amoderia"}, {"from":"J131142","to":"HL-VZX"},
    {"from":"J131142","to":"J133011"},
    {"from":"J133011","to":"J133525"}, {"from":"J133011","to":"J151319"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 25
# Safest leaves via Orduin (0.76 highsec), not the shortest Lashkai exit.
jsonpath "$.path[1].system" == "Orduin"
# Every K-space hop on the safest route is highsec.
jsonpath "$.path[*].sec_class" not contains "Lowsec"
jsonpath "$.path[*].sec_class" not contains "Nullsec"

# ── deep wormhole -> K-space: J133525 (far end of chain A) -> Adar = 17 ───────
# 3 WH hops out of chain A, K-space gates, then 1 WH into Adar's branch.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J133525",
  "to": "Adar",
  "use_wormholes": true,
  "connections": [
    {"from":"J133011","to":"J133525"}, {"from":"J131142","to":"J133011"},
    {"from":"J120455","to":"J131142"}, {"from":"J120455","to":"Lashkai"},
    {"from":"J172840","to":"J213956"}, {"from":"J213956","to":"J161411"},
    {"from":"J161411","to":"J122832"}, {"from":"J122832","to":"Adar"},
    {"from":"J161411","to":"J131204"}, {"from":"J131204","to":"Martha"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 17
jsonpath "$.path[0].system" == "J133525"
jsonpath "$.path[-1].system" == "Adar"

# ── J172840 -> Amarr via the Adar branch = 10 ────────────────────────────────
# 4 wormhole hops out to Adar (0.8, Tash-Murkon), then 6 gates to Amarr.
POST {{base_url}}/route/gate
Content-Type: application/json
{
  "from": "J172840",
  "to": "Amarr",
  "use_wormholes": true,
  "connections": [
    {"from":"J172840","to":"J213956"}, {"from":"J213956","to":"J161411"},
    {"from":"J161411","to":"J122832"}, {"from":"J122832","to":"Adar"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.jumps" == 10
jsonpath "$.path[4].system" == "Adar"
jsonpath "$.path[4].via" == "wormhole"
jsonpath "$.path[-1].system" == "Amarr"
jsonpath "$.path[-1].via" == "stargate"

# Wormhole-chain routing over a real, scanned wormhole map — fan-out shape.
#
# The endpoint is a fan-out: a shared header (including the scanned `connections[]`
# chain, stated once) plus `to: [...]`, returning `{ from, results: [...] }`.
# Single-destination requests assert against `$.results[0]`; the multi-destination
# request near the end fans one source out to several hubs over one shared chain.
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
#   PALU_CACHE_DIR=tests/fixtures/sde PALU_SDE_RELOAD_INTERVAL_SECS=0 \
#   PALU_EVE_SCOUT_INTERVAL_SECS=0 cargo run

# ── J120455 -> J172840: 14 jumps, bridging the two chains via K-space ─────────
# J120455 -(WH)-> Lashkai -(9 gates through Amarr)-> Martha -(4 WH)-> J172840.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["J172840"],
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"Orduin"},
    {"type":"wormhole","from":"J120455","to":"DDI-B7"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"Amoderia"}, {"type":"wormhole","from":"J131142","to":"HL-VZX"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"},
    {"type":"wormhole","from":"J172840","to":"J213956"}, {"type":"wormhole","from":"J172840","to":"O-TVTD"},
    {"type":"wormhole","from":"J213956","to":"J113453"}, {"type":"wormhole","from":"J213956","to":"J161411"},
    {"type":"wormhole","from":"J113453","to":"J162132"}, {"type":"wormhole","from":"J162132","to":"J112241"},
    {"type":"wormhole","from":"J113453","to":"QPTT-F"}, {"type":"wormhole","from":"J113453","to":"AU2V-J"},
    {"type":"wormhole","from":"J161411","to":"J145155"}, {"type":"wormhole","from":"J161411","to":"J131204"},
    {"type":"wormhole","from":"J161411","to":"J122832"},
    {"type":"wormhole","from":"J145155","to":"J131852"}, {"type":"wormhole","from":"J145155","to":"Atai"},
    {"type":"wormhole","from":"J145155","to":"J140810"}, {"type":"wormhole","from":"J145155","to":"Iosantin"},
    {"type":"wormhole","from":"J145155","to":"J105415"},
    {"type":"wormhole","from":"J131204","to":"J222104"}, {"type":"wormhole","from":"J131204","to":"Martha"},
    {"type":"wormhole","from":"J122832","to":"J113925"}, {"type":"wormhole","from":"J122832","to":"Adar"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 14
jsonpath "$.results[0].path[0].system" == "J120455"
jsonpath "$.results[0].path[0].via" == "start"
jsonpath "$.results[0].path[-1].system" == "J172840"
jsonpath "$.results[0].path[-1].via" == "wormhole"
# The route leaves chain A via a wormhole into K-space (Lashkai)...
jsonpath "$.results[0].path[1].system" == "Lashkai"
jsonpath "$.results[0].path[1].via" == "wormhole"
# ...crosses real space on stargates...
jsonpath "$.results[0].path[*].via" contains "stargate"
# ...and re-enters chain B's wormholes via Martha.
jsonpath "$.results[0].path[*].system" contains "Martha"

# ── reverse is symmetric: J172840 -> J120455 is also 14 ───────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J172840",
  "to": ["J120455"],
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"},
    {"type":"wormhole","from":"J172840","to":"J213956"},
    {"type":"wormhole","from":"J213956","to":"J113453"}, {"type":"wormhole","from":"J213956","to":"J161411"},
    {"type":"wormhole","from":"J161411","to":"J131204"}, {"type":"wormhole","from":"J131204","to":"Martha"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 14

# ── no connections: the two chains are disconnected -> in-slot unreachable ────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["J172840"]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].error" == "unreachable"

# ── only chain A supplied: can reach its exits, but not J172840 ───────────────
# Without chain B's wormholes there is no way back into J-space on the far side,
# so the destination is unreachable (in-slot) even though K-space is traversable.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["J172840"],
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"Orduin"},
    {"type":"wormhole","from":"J120455","to":"DDI-B7"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"Amoderia"}, {"type":"wormhole","from":"J131142","to":"HL-VZX"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].error" == "unreachable"

# ── a wormhole NOT on the map is unreachable even with the full chain ─────────
# J105443 is a real J-space system with no gates and no overlay edge, so no
# amount of chain connections can reach it (in-slot unreachable).
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["J105443"],
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"J133011"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].error" == "unreachable"

# ── J120455 -> Jita, shortest = 13 ───────────────────────────────────────────
# Shortest exits deep into chain A: J120455 -(WH)-> J131142 -(WH)-> Amoderia
# (0.4, Verge Vendor), then 11 gates to Jita.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["Jita"],
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"Orduin"},
    {"type":"wormhole","from":"J120455","to":"DDI-B7"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"Amoderia"}, {"type":"wormhole","from":"J131142","to":"HL-VZX"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 13
jsonpath "$.results[0].path[0].system" == "J120455"
jsonpath "$.results[0].path[1].via" == "wormhole"
jsonpath "$.results[0].path[2].system" == "Amoderia"
jsonpath "$.results[0].path[-1].system" == "Jita"
jsonpath "$.results[0].path[-1].via" == "stargate"

# ── J120455 -> Jita, safest = 25: all-highsec via Orduin ─────────────────────
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["Jita"],
  "preference": "safest",
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"Orduin"},
    {"type":"wormhole","from":"J120455","to":"DDI-B7"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"Amoderia"}, {"type":"wormhole","from":"J131142","to":"HL-VZX"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 25
# Safest leaves via Orduin (0.76 highsec), not the shortest Lashkai exit.
jsonpath "$.results[0].path[1].system" == "Orduin"
# Every K-space hop on the safest route is highsec.
jsonpath "$.results[0].path[*].sec_class" not contains "Lowsec"
jsonpath "$.results[0].path[*].sec_class" not contains "Nullsec"

# ── deep wormhole -> K-space: J133525 (far end of chain A) -> Adar = 17 ───────
# 3 WH hops out of chain A, K-space gates, then 1 WH into Adar's branch.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J133525",
  "to": ["Adar"],
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J120455","to":"J131142"}, {"type":"wormhole","from":"J120455","to":"Lashkai"},
    {"type":"wormhole","from":"J172840","to":"J213956"}, {"type":"wormhole","from":"J213956","to":"J161411"},
    {"type":"wormhole","from":"J161411","to":"J122832"}, {"type":"wormhole","from":"J122832","to":"Adar"},
    {"type":"wormhole","from":"J161411","to":"J131204"}, {"type":"wormhole","from":"J131204","to":"Martha"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 17
jsonpath "$.results[0].path[0].system" == "J133525"
jsonpath "$.results[0].path[-1].system" == "Adar"

# ── J172840 -> Amarr via the Adar branch = 10 ────────────────────────────────
# 4 wormhole hops out to Adar (0.8, Tash-Murkon), then 6 gates to Amarr.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J172840",
  "to": ["Amarr"],
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J172840","to":"J213956"}, {"type":"wormhole","from":"J213956","to":"J161411"},
    {"type":"wormhole","from":"J161411","to":"J122832"}, {"type":"wormhole","from":"J122832","to":"Adar"}
  ]
}

HTTP 200
[Asserts]
jsonpath "$.results[0].jumps" == 10
jsonpath "$.results[0].path[4].system" == "Adar"
jsonpath "$.results[0].path[4].via" == "wormhole"
jsonpath "$.results[0].path[-1].system" == "Amarr"
jsonpath "$.results[0].path[-1].via" == "stargate"

# ── MULTI-DESTINATION FAN-OUT over chain A (shared chain stated once) ─────────
# One source (J120455), one shared scanned chain, several destinations: J172840
# (the far chain, 14), Jita (shortest, 13), Amoderia (a chain-A exit, 2), plus a
# deliberately unreachable hub (J105443) that fails only in its own slot.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
{
  "from": "J120455",
  "to": ["J172840", "Jita", "Amoderia", "J105443"],
  "preference": "shortest",
  "use_wormholes": true,
  "connections": [
    {"type":"wormhole","from":"J120455","to":"Lashkai"}, {"type":"wormhole","from":"J120455","to":"Orduin"},
    {"type":"wormhole","from":"J120455","to":"DDI-B7"}, {"type":"wormhole","from":"J120455","to":"J131142"},
    {"type":"wormhole","from":"J131142","to":"Amoderia"}, {"type":"wormhole","from":"J131142","to":"HL-VZX"},
    {"type":"wormhole","from":"J131142","to":"J133011"},
    {"type":"wormhole","from":"J133011","to":"J133525"}, {"type":"wormhole","from":"J133011","to":"J151319"},
    {"type":"wormhole","from":"J172840","to":"J213956"}, {"type":"wormhole","from":"J172840","to":"O-TVTD"},
    {"type":"wormhole","from":"J213956","to":"J113453"}, {"type":"wormhole","from":"J213956","to":"J161411"},
    {"type":"wormhole","from":"J161411","to":"J131204"}, {"type":"wormhole","from":"J131204","to":"Martha"}
  ]
}

HTTP 200
[Asserts]
# `from` echoed once; one entry per destination, in request order.
jsonpath "$.from" == "J120455"
jsonpath "$.results" count == 4
jsonpath "$.results[0].to" == "J172840"
jsonpath "$.results[0].jumps" == 14
jsonpath "$.results[1].to" == "Jita"
jsonpath "$.results[1].jumps" == 13
jsonpath "$.results[2].to" == "Amoderia"
jsonpath "$.results[2].jumps" == 2
jsonpath "$.results[2].path[*].via" contains "wormhole"
# The unreachable hub fails in-slot; the others still resolved.
jsonpath "$.results[3].to" == "J105443"
jsonpath "$.results[3].error" == "unreachable"

# ── the committed load body: every destination resolves ──────────────────────
# This is the same body `just load-test` fires (tests/load/fanout-wh.json): one
# source, the full 29-edge chain stated once, a diverse 18-destination list whose
# routes span 1..=39 jumps. Pinning "every destination resolves" here keeps the
# load body honest (a fixture refresh that breaks a route fails CI, not just the
# load run) and shares the body between the two.
POST {{base_url}}/api/v1/route/system
Content-Type: application/json
file,tests/load/fanout-wh.json;

HTTP 200
[Asserts]
jsonpath "$.from" == "J120455"
jsonpath "$.results" count == 18
# No destination errored — every entry carries a route (jumps), none an error.
jsonpath "$.results[*].error" not exists
jsonpath "$.results[*].jumps" count == 18
# Spot-check the span endpoints: a 1-jump in-chain hop and the deep ~38-jump hub.
jsonpath "$.results[0].to" == "J131142"
jsonpath "$.results[0].jumps" == 1
jsonpath "$.results[16].to" == "319-3D"
jsonpath "$.results[16].jumps" == 38

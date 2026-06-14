# GET /health — flat status document, no auth, no envelope.
GET {{base_url}}/health

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.status" == "ok"
jsonpath "$.systems" > 0
jsonpath "$.edges" > 0
# The hull catalog loaded from the fixture's type files.
jsonpath "$.hull_count" > 0
# Freshness fields are null/0 until the first reload swap / EVE-Scout fetch.
jsonpath "$.last_sde_reload_at" == null
jsonpath "$.sig_count" == 0
jsonpath "$.last_evescout_fetch_at" == null
# /health is intentionally un-enveloped (no `data` wrapper).
jsonpath "$.data" not exists

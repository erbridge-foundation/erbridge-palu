# OpenAPI JSON and Swagger UI are served unconditionally (docker-internal, no auth).
GET {{base_url}}/api-docs/openapi.json

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.openapi" startsWith "3."
jsonpath "$.paths['/api/v1/route/system']" exists
jsonpath "$.paths['/api/v1/route/blops']" exists
jsonpath "$.paths['/api/v1/route/range']" exists
jsonpath "$.paths['/health']" exists
# Typed-connection overlay: the Connection schema has a required `type`
# discriminator over the bridge/wormhole enum, and the system-route request
# carries the independent `use_bridges` flag.
jsonpath "$.components.schemas.Connection.required" contains "type"
jsonpath "$.components.schemas.ConnectionKind.enum" contains "bridge"
jsonpath "$.components.schemas.ConnectionKind.enum" contains "wormhole"
jsonpath "$.components.schemas.GateRouteRequest.properties.use_bridges" exists


# Swagger UI is reachable (it serves an HTML page).
GET {{base_url}}/swagger-ui/
[Options]
location: true

HTTP 200
[Asserts]
header "Content-Type" contains "text/html"

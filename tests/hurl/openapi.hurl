# OpenAPI JSON and Swagger UI are served unconditionally (docker-internal, no auth).
GET {{base_url}}/api-docs/openapi.json

HTTP 200
[Asserts]
header "Content-Type" contains "application/json"
jsonpath "$.openapi" startsWith "3."
jsonpath "$.paths['/api/v1/route/system']" exists
jsonpath "$.paths['/health']" exists


# Swagger UI is reachable (it serves an HTML page).
GET {{base_url}}/swagger-ui/
[Options]
location: true

HTTP 200
[Asserts]
header "Content-Type" contains "text/html"

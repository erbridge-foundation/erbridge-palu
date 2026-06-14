# health-and-openapi Specification

## Purpose

Expose operational observability and API documentation: an unauthenticated health
endpoint reporting graph and freshness state, plus a generated OpenAPI document
and interactive Swagger UI.

## Requirements

### Requirement: Health endpoint

The service SHALL expose `GET /health` returning status, the loaded SDE build
number (`sde_version`), system and edge counts, the loaded hull count
(`hull_count`), the timestamp of the last successful SDE hot-reload swap
(`last_sde_reload_at`, `null` until the first real swap), and EVE-Scout freshness
(`sig_count` and `last_evescout_fetch_at`, `0`/`null` until the first successful
fetch). The health endpoint SHALL require no authentication.

#### Scenario: Health reflects loaded graph

- **WHEN** the graph is loaded and `GET /health` is called
- **THEN** the response reports `status` ok with the `sde_version`, non-zero system
  and edge counts, and a non-zero `hull_count`

#### Scenario: Freshness fields before first refresh

- **WHEN** the service has loaded from cache but never performed a hot-reload swap
  and never reached EVE-Scout
- **THEN** `last_sde_reload_at` is `null` and the EVE-Scout fields indicate no
  fetch yet

### Requirement: OpenAPI specification and Swagger UI

The service SHALL generate an OpenAPI 3.1 document from its handlers and DTOs and
serve it as JSON, and SHALL serve an interactive Swagger UI. Both SHALL be served
unconditionally (no authentication), since the service is docker-internal. All
request and response DTOs SHALL be represented in the generated schema, including
reserved optional fields.

#### Scenario: OpenAPI document is served

- **WHEN** the OpenAPI JSON endpoint is requested
- **THEN** a valid OpenAPI 3.1 document describing `/api/v1/route/system` and `/health` is
  returned

#### Scenario: Swagger UI is reachable

- **WHEN** the Swagger UI path is requested
- **THEN** the interactive documentation page loads

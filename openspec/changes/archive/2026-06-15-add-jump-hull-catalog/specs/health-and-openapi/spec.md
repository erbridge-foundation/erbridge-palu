## MODIFIED Requirements

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

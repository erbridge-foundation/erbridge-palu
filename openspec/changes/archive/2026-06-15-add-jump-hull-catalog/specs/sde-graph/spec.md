## MODIFIED Requirements

### Requirement: SDE load and disk cache

The service SHALL obtain the EVE Static Data Export by reading CCP's build-number
manifest, downloading the matching per-build ZIP, extracting only
`mapSolarSystems.jsonl`, `mapStargates.jsonl`, `types.jsonl`, and
`typeDogma.jsonl`, and caching the extracted files on disk so restarts do not
require re-download. Cache writes SHALL be atomic (write to a temporary path, then
rename). The cache directory SHALL default to the platform cache location and be
overridable via `GEODESIC_CACHE_DIR`. Because `types.jsonl` is large (every item
in EVE), parsing it SHALL stream and filter during the read, retaining only
jump-capable hulls, and SHALL NOT materialise the full file in memory.

#### Scenario: Cold start with no cache

- **WHEN** the service starts and no usable cache exists
- **THEN** it fetches the manifest, downloads and extracts the current build, writes
  the files atomically to the cache, and builds the in-memory graph and hull catalog
- **AND** if this initial load fails, startup fails (fatal)

#### Scenario: Warm start from cache

- **WHEN** the service starts and a complete, readable cached build exists
- **THEN** it loads `GraphData` (including the hull catalog) from the cached files
  without re-downloading

#### Scenario: ZIP-slip defence

- **WHEN** an archive entry has a `..` path segment or an absolute path
- **THEN** that entry is rejected and not written to the cache

#### Scenario: Type data is required for a complete cached build

- **WHEN** a cached build directory is missing any required file, including
  `types.jsonl` or `typeDogma.jsonl`
- **THEN** the build is treated as incomplete and is re-fetched rather than loaded
  partially

### Requirement: SDE hot-reload

The service SHALL hold `GraphData` behind an `ArcSwap` and run a background task
that polls the SDE manifest on an interval (`GEODESIC_SDE_RELOAD_INTERVAL_SECS`,
default 3600; `0` disables). When a newer build number is published, the task
SHALL build a complete new `GraphData` — including the hull catalog — in memory and
only then atomically swap it in. The map graph and hull catalog in any live
snapshot SHALL always originate from the same SDE build (no skew). Requests in
flight SHALL continue using the snapshot they loaded.

#### Scenario: Newer build triggers an atomic swap

- **WHEN** the poller observes a build number newer than the loaded one
- **THEN** it builds the new graph and hull catalog fully, swaps them in atomically
  as one unit, and updates the disk cache, pruning the previous build only after
  the new one is fully extracted

#### Scenario: Reload failure is non-fatal

- **WHEN** a reload attempt fails (download, extract, or parse error)
- **THEN** the error is logged and the service keeps serving the previously loaded
  graph and catalog

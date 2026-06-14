# sde-graph Specification

## Purpose

Obtain the EVE Static Data Export, cache it on disk, build and maintain the
in-memory gate graph and spatial index, and hot-reload them when a newer SDE
build is published.

## Requirements

### Requirement: SDE load and disk cache

The service SHALL obtain the EVE Static Data Export by reading CCP's build-number
manifest, downloading the matching per-build ZIP, extracting only
`mapSolarSystems.jsonl` and `mapStargates.jsonl`, and caching the extracted files
on disk so restarts do not require re-download. Cache writes SHALL be atomic
(write to a temporary path, then rename). The cache directory SHALL default to the
platform cache location and be overridable via `GEODESIC_CACHE_DIR`.

#### Scenario: Cold start with no cache

- **WHEN** the service starts and no usable cache exists
- **THEN** it fetches the manifest, downloads and extracts the current build, writes
  the files atomically to the cache, and builds the in-memory graph
- **AND** if this initial load fails, startup fails (fatal)

#### Scenario: Warm start from cache

- **WHEN** the service starts and a complete, readable cached build exists
- **THEN** it loads `GraphData` from the cached files without re-downloading

#### Scenario: ZIP-slip defence

- **WHEN** an archive entry has a `..` path segment or an absolute path
- **THEN** that entry is rejected and not written to the cache

### Requirement: In-memory gate graph and lookups

The service SHALL build an in-memory `GraphData` containing one `System` per solar
system, an undirected gate graph deduplicated to at most one edge per system pair,
a system-id→index map, and a case-insensitive name→index map. System security
class SHALL be derived from raw `securityStatus` (highsec iff non-wormhole and
`securityStatus >= 0.45`), and wormhole systems SHALL be identified by
`region_id >= 11_000_000`.

#### Scenario: Directed stargate pairs become one undirected edge

- **WHEN** the SDE contains the two directed stargates linking systems A and B
- **THEN** the gate graph contains exactly one undirected A–B edge

#### Scenario: Name lookup is case-insensitive

- **WHEN** a caller resolves a system by name in any letter case
- **THEN** the same system index is returned

### Requirement: kd-tree spatial index

The service SHALL build a 3-D kd-tree spatial index over K-space system positions
as part of `GraphData` construction. This index is reserved for future
light-year-distance queries and is not queried by any foundation endpoint.

#### Scenario: Spatial index is built alongside the graph

- **WHEN** `GraphData` is constructed (on startup or hot-reload)
- **THEN** the kd-tree is populated with K-space system coordinates so it is
  available without re-touching graph construction later

### Requirement: SDE hot-reload

The service SHALL hold `GraphData` behind an `ArcSwap` and run a background task
that polls the SDE manifest on an interval (`GEODESIC_SDE_RELOAD_INTERVAL_SECS`,
default 3600; `0` disables). When a newer build number is published, the task
SHALL build a complete new `GraphData` in memory and only then atomically swap it
in. Requests in flight SHALL continue using the snapshot they loaded.

#### Scenario: Newer build triggers an atomic swap

- **WHEN** the poller observes a build number newer than the loaded one
- **THEN** it builds the new graph fully, swaps it in atomically, and updates the
  disk cache, pruning the previous build only after the new one is fully extracted

#### Scenario: Reload failure is non-fatal

- **WHEN** a reload attempt fails (download, extract, or parse error)
- **THEN** the error is logged and the service keeps serving the previously loaded
  graph

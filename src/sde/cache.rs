//! SDE disk cache: manifest fetch, download + extract (ZIP-slip defended,
//! atomic writes), metadata bookkeeping, and old-build pruning.

use std::{
    io::{BufReader, Read, Seek},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::parse::{ParseError, parse_gate_pairs, parse_hull_catalog, parse_systems};
use crate::model::RawSdeData;

const LATEST_URL: &str = "https://developers.eveonline.com/static-data/tranquility/latest.jsonl";

fn zip_url(build: u64) -> String {
    format!(
        "https://developers.eveonline.com/static-data/tranquility/eve-online-static-data-{build}-jsonl.zip"
    )
}

/// Only the files we deserialize. Add more here (and matching types in
/// `types.rs`) if a future change needs region/constellation lookup.
///
/// `types.jsonl` and `typeDogma.jsonl` feed the hull catalog. `types.jsonl` is
/// ~150 MB (every item in EVE); it is stream-filtered during parse and never
/// materialised in full — see `parse::parse_hull_catalog`.
const FILES: &[&str] = &[
    "mapSolarSystems.jsonl",
    "mapStargates.jsonl",
    "types.jsonl",
    "typeDogma.jsonl",
];

/// CCP's `latest.jsonl` manifest entry.
#[derive(Debug, Clone, Deserialize)]
struct LatestManifest {
    #[serde(rename = "buildNumber")]
    build_number: u64,
    #[serde(rename = "releaseDate")]
    release_date: DateTime<Utc>,
}

/// Pointer file at `<cache>/metadata.json`. Identifies the current build subdir.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CacheMetadata {
    pub build_number: u64,
    pub release_date: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,
}

pub struct SdeCache {
    pub root: PathBuf,
}

impl SdeCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn metadata_path(&self) -> PathBuf {
        self.root.join("metadata.json")
    }

    pub fn build_dir(&self, build: u64) -> PathBuf {
        self.root.join(build.to_string())
    }

    /// The build recorded in metadata.json, iff all its files exist on disk.
    pub fn current_metadata(&self) -> Option<CacheMetadata> {
        let bytes = std::fs::read(self.metadata_path()).ok()?;
        let meta: CacheMetadata = serde_json::from_slice(&bytes).ok()?;
        if !self.build_files_present(meta.build_number) {
            return None;
        }
        Some(meta)
    }

    fn build_files_present(&self, build: u64) -> bool {
        let dir = self.build_dir(build);
        FILES.iter().all(|f| dir.join(f).exists())
    }

    fn write_metadata(&self, meta: &CacheMetadata) -> Result<()> {
        let dest = self.metadata_path();
        let tmp = self.root.join("metadata.json.tmp");
        std::fs::create_dir_all(&self.root).context("creating cache root")?;
        std::fs::write(&tmp, serde_json::to_vec_pretty(meta)?)
            .context("writing metadata tmp file")?;
        std::fs::rename(&tmp, &dest).context("renaming metadata tmp file")?;
        Ok(())
    }

    /// Load `RawSdeData` from a specific cached build. Verifies all required
    /// files are present first so a caller that bypasses `current_metadata`
    /// gets a clear error rather than a partial parse.
    pub fn load_build(&self, build: u64) -> Result<RawSdeData> {
        let dir = self.build_dir(build);
        if !self.build_files_present(build) {
            anyhow::bail!(
                "build {build} cache is incomplete (missing one of {:?} in {})",
                FILES,
                dir.display()
            );
        }
        let systems = parse_file(&dir, "mapSolarSystems.jsonl", parse_systems)?;
        let gate_pairs = parse_file(&dir, "mapStargates.jsonl", parse_gate_pairs)?;
        let hulls = load_hull_catalog(&dir)?;
        Ok(RawSdeData {
            systems,
            gate_pairs,
            hulls,
        })
    }

    /// Garbage-collect any build subdirs other than `keep`.
    pub fn prune_other_builds(&self, keep: u64) -> Result<()> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Only prune dirs whose names parse as a build number.
            if let Ok(b) = name.parse::<u64>()
                && b != keep
            {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    warn!(build = b, error = %e, "failed to prune old build dir");
                } else {
                    info!(build = b, "pruned old SDE build");
                }
            }
        }
        Ok(())
    }
}

fn parse_file<T, F>(dir: &Path, name: &str, f: F) -> Result<T>
where
    F: Fn(BufReader<std::fs::File>) -> std::result::Result<T, ParseError>,
{
    let path = dir.join(name);
    let file = std::fs::File::open(&path).with_context(|| format!("opening cached {name}"))?;
    f(BufReader::new(file)).with_context(|| format!("parsing {name}"))
}

/// Build the raw hull catalog by stream-parsing `types.jsonl` and joining
/// `typeDogma.jsonl`. Both readers are opened here (rather than via `parse_file`)
/// because the parser needs both files together to perform the join.
fn load_hull_catalog(dir: &Path) -> Result<crate::model::RawHullCatalog> {
    let types_path = dir.join("types.jsonl");
    let dogma_path = dir.join("typeDogma.jsonl");
    let types = std::fs::File::open(&types_path).context("opening cached types.jsonl")?;
    let dogma = std::fs::File::open(&dogma_path).context("opening cached typeDogma.jsonl")?;
    parse_hull_catalog(BufReader::new(types), BufReader::new(dogma))
        .context("parsing types.jsonl/typeDogma.jsonl")
}

/// Resolve the cache directory: `PALU_CACHE_DIR` override → platform cache.
pub fn resolve_cache_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("PALU_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dirs = directories::ProjectDirs::from("", "", "erbridge-palu")
        .context("could not determine platform cache directory")?;
    Ok(dirs.cache_dir().join("sde"))
}

/// Fetch `latest.jsonl` and return the current manifest.
async fn fetch_latest_build(client: &reqwest::Client) -> Result<LatestManifest> {
    let resp = client
        .get(LATEST_URL)
        .send()
        .await
        .context("fetching latest.jsonl")?
        .error_for_status()
        .context("latest.jsonl returned error status")?;
    let text = resp.text().await.context("reading latest.jsonl body")?;
    // JSONL, but in practice a single object — take the first non-empty line.
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .context("latest.jsonl was empty")?;
    let manifest: LatestManifest = serde_json::from_str(line).context("parsing latest.jsonl")?;
    Ok(manifest)
}

/// Download the build ZIP and stream the required files into
/// `<cache>/<build>/`, each via `*.tmp` then atomic rename.
pub async fn fetch_and_extract_build(
    client: &reqwest::Client,
    cache: &SdeCache,
    build: u64,
) -> Result<()> {
    let url = zip_url(build);
    info!(build, %url, "downloading SDE");
    let bytes = client
        .get(&url)
        .send()
        .await
        .context("sending SDE zip request")?
        .error_for_status()
        .context("SDE zip returned error status")?
        .bytes()
        .await
        .context("reading SDE zip body")?;
    info!(build, bytes = bytes.len(), "SDE downloaded");

    let dir = cache.build_dir(build);
    std::fs::create_dir_all(&dir).context("creating build dir")?;
    extract_files(std::io::Cursor::new(bytes), &dir)?;
    Ok(())
}

/// Extract the required JSONL files from the ZIP into `dest_dir`. Each is
/// written to a `.tmp` file then renamed atomically. Entries are matched by
/// exact basename against `FILES`. Entries whose paths attempt to escape
/// `dest_dir` (`..` segments or absolute paths) are rejected — explicit is
/// safer than relying on the basename strip.
fn extract_files<R: Read + Seek>(reader: R, dest_dir: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader).context("opening ZIP archive")?;
    let mut found = std::collections::HashSet::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("reading zip entry")?;
        let raw_name = entry.name().to_string();

        if is_unsafe_zip_path(&raw_name) {
            warn!(name = %raw_name, "rejecting unsafe ZIP entry (path traversal)");
            continue;
        }

        let basename = Path::new(&raw_name)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !FILES.contains(&basename) {
            continue;
        }
        let basename = basename.to_string();
        let dest = dest_dir.join(&basename);
        let tmp = dest_dir.join(format!("{basename}.tmp"));
        let mut out =
            std::fs::File::create(&tmp).with_context(|| format!("creating {basename}.tmp"))?;
        std::io::copy(&mut entry, &mut out).with_context(|| format!("writing {basename}.tmp"))?;
        out.sync_all().ok();
        std::fs::rename(&tmp, &dest).with_context(|| format!("renaming {basename}.tmp"))?;
        found.insert(basename);
    }

    for name in FILES {
        if !found.contains(*name) {
            anyhow::bail!("expected file {name} not found in SDE zip");
        }
    }
    Ok(())
}

/// True if a ZIP entry name is unsafe to extract (absolute path or `..`).
fn is_unsafe_zip_path(name: &str) -> bool {
    if name.starts_with('/') || name.starts_with('\\') {
        return true;
    }
    // Windows-style drive letter, e.g. `C:\evil.txt`.
    if name.len() >= 2 && name.as_bytes()[1] == b':' && name.as_bytes()[0].is_ascii_alphabetic() {
        return true;
    }
    Path::new(name)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Bring the cache to a usable state and return loaded data + active metadata.
///
/// On startup we want to serve the latest SDE build, so the manifest is
/// consulted up front:
///
/// * No valid cache → fetch synchronously. Failure is fatal — we can't serve
///   without a graph.
/// * Valid cache present → check `latest.jsonl`. If a newer build exists,
///   download it before serving. If the manifest check (or the download of the
///   newer build) fails, fall back to the cached build so a network blip
///   doesn't prevent startup; the hot-reload task will catch up later.
pub async fn ensure_cache(
    client: &reqwest::Client,
    cache: &SdeCache,
) -> Result<(RawSdeData, CacheMetadata)> {
    std::fs::create_dir_all(&cache.root).context("creating cache root")?;

    let Some(meta) = cache.current_metadata() else {
        info!("no SDE cache on disk; downloading the latest build before serving");
        let latest = fetch_latest_build(client)
            .await
            .context("fetching latest SDE manifest")?;
        info!(
            build = latest.build_number,
            released = %latest.release_date,
            "latest SDE build resolved; downloading"
        );
        let new_meta = fetch_and_install(client, cache, latest).await?;
        let data = cache.load_build(new_meta.build_number)?;
        info!(
            build = new_meta.build_number,
            "cold SDE fetch complete; serving fresh build"
        );
        return Ok((data, new_meta));
    };

    // A valid cache exists; make sure it's the latest build before serving.
    info!(
        cached = meta.build_number,
        fetched_at = %meta.fetched_at,
        "found cached SDE; checking the manifest for a newer build before serving"
    );
    match check_and_refresh(client, cache, meta.build_number).await {
        Ok(RefreshOutcome::UpToDate) => {
            info!(
                build = meta.build_number,
                fetched_at = %meta.fetched_at,
                "cached SDE is already the latest build; loading from cache"
            );
            let data = cache.load_build(meta.build_number)?;
            Ok((data, meta))
        }
        Ok(RefreshOutcome::Updated {
            data,
            meta: new_meta,
        }) => {
            info!(
                previous = meta.build_number,
                build = new_meta.build_number,
                released = ?new_meta.release_date,
                "upgraded cached SDE to the latest build at startup"
            );
            Ok((data, new_meta))
        }
        Err(e) => {
            warn!(
                error = %e,
                build = meta.build_number,
                "could not confirm the latest SDE build at startup (offline?); \
                 serving the cached build — hot-reload will retry"
            );
            let data = cache.load_build(meta.build_number)?;
            Ok((data, meta))
        }
    }
}

/// `PALU_SDE_DIR` override, if set: a directory holding pre-extracted
/// `mapSolarSystems.jsonl` / `mapStargates.jsonl` to load directly, skipping
/// the manifest/download path entirely. Intended for local dev and offline
/// tests (e.g. the HURL server). Build number is reported as `0`.
pub fn resolve_sde_dir() -> Option<PathBuf> {
    std::env::var("PALU_SDE_DIR").ok().map(PathBuf::from)
}

/// Load `RawSdeData` directly from a directory containing the two JSONL files
/// (no build subdir, no metadata, no network). Returns a synthetic metadata
/// with build number `0`.
pub fn load_from_dir(dir: &Path) -> Result<(RawSdeData, CacheMetadata)> {
    info!(dir = %dir.display(), "loading SDE from PALU_SDE_DIR (offline)");
    for f in FILES {
        if !dir.join(f).exists() {
            anyhow::bail!("PALU_SDE_DIR {} is missing {f}", dir.display());
        }
    }
    let systems = parse_file(dir, "mapSolarSystems.jsonl", parse_systems)?;
    let gate_pairs = parse_file(dir, "mapStargates.jsonl", parse_gate_pairs)?;
    let hulls = load_hull_catalog(dir)?;
    let meta = CacheMetadata {
        build_number: 0,
        release_date: None,
        fetched_at: Utc::now(),
    };
    Ok((
        RawSdeData {
            systems,
            gate_pairs,
            hulls,
        },
        meta,
    ))
}

/// Refresh outcome from `check_and_refresh`. The caller decides whether to
/// swap the live `Arc<GraphData>`.
pub enum RefreshOutcome {
    /// Cache was already current — `current_build` matched the manifest.
    UpToDate,
    /// A new build was downloaded, extracted and made current.
    Updated {
        data: RawSdeData,
        meta: CacheMetadata,
    },
}

/// Best-effort freshness check used by the hot-reload background task. Fetches
/// the manifest, compares to `current_build`, and (if newer) downloads +
/// extracts + atomically promotes the new build. Network errors surface as
/// `Err` so the caller can log and retry.
pub async fn check_and_refresh(
    client: &reqwest::Client,
    cache: &SdeCache,
    current_build: u64,
) -> Result<RefreshOutcome> {
    let latest = fetch_latest_build(client)
        .await
        .context("fetching latest SDE manifest")?;
    if latest.build_number == current_build {
        return Ok(RefreshOutcome::UpToDate);
    }
    info!(
        cached = current_build,
        latest = latest.build_number,
        "newer SDE available; refreshing"
    );
    let new_meta = fetch_and_install(client, cache, latest).await?;
    let data = cache.load_build(new_meta.build_number)?;
    Ok(RefreshOutcome::Updated {
        data,
        meta: new_meta,
    })
}

async fn fetch_and_install(
    client: &reqwest::Client,
    cache: &SdeCache,
    latest: LatestManifest,
) -> Result<CacheMetadata> {
    // Extract the new build fully before touching metadata or pruning, so the
    // previous build's files survive until the new one is complete.
    fetch_and_extract_build(client, cache, latest.build_number)
        .await
        .context("fetching SDE zip")?;
    let new_meta = CacheMetadata {
        build_number: latest.build_number,
        release_date: Some(latest.release_date),
        fetched_at: Utc::now(),
    };
    cache.write_metadata(&new_meta)?;
    let _ = cache.prune_other_builds(latest.build_number);
    info!(
        build = new_meta.build_number,
        dir = %cache.build_dir(new_meta.build_number).display(),
        "SDE build installed and made current"
    );
    Ok(new_meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn make_zip_with_files(files: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in files {
            zip.start_file(format!("Latest/{name}"), opts).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Writes entry names verbatim (no `Latest/` prefix) so the ZIP-slip test
    /// can produce traversal entries.
    fn make_zip_with_files_raw(files: &[(&str, &[u8])]) -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in files {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Write a minimal but complete four-file build into `dir`: one system, no
    /// gates, one published jump hull (attr 867 = 4.0) plus the JDC skill (attr
    /// 870 = 20.0). Used by the offline-load tests.
    fn write_minimal_build(dir: &Path) {
        std::fs::write(
            dir.join("mapSolarSystems.jsonl"),
            "{\"_key\":30000142,\"name\":{\"en\":\"Jita\"},\"securityStatus\":0.95,\"regionID\":10000002,\"constellationID\":1,\"position\":{\"x\":0,\"y\":0,\"z\":0}}\n",
        )
        .unwrap();
        std::fs::write(dir.join("mapStargates.jsonl"), "").unwrap();
        std::fs::write(
            dir.join("types.jsonl"),
            "{\"_key\":22430,\"groupID\":898,\"name\":{\"en\":\"Sin\"},\"published\":true}\n{\"_key\":21611,\"groupID\":257,\"name\":{\"en\":\"Jump Drive Calibration\"},\"published\":true}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("typeDogma.jsonl"),
            "{\"_key\":22430,\"dogmaAttributes\":[{\"attributeID\":867,\"value\":4.0}]}\n{\"_key\":21611,\"dogmaAttributes\":[{\"attributeID\":870,\"value\":20.0}]}\n",
        )
        .unwrap();
    }

    #[test]
    fn load_from_dir_reads_jsonl_directly() {
        let dir = tempfile::tempdir().unwrap();
        write_minimal_build(dir.path());
        let (raw, meta) = load_from_dir(dir.path()).unwrap();
        assert_eq!(raw.systems.len(), 1);
        assert_eq!(raw.systems[0].name, "Jita");
        assert_eq!(meta.build_number, 0);
        // The two type files are parsed and joined into the hull catalog.
        assert_eq!(raw.hulls.hulls.len(), 1);
        assert_eq!(raw.hulls.hulls[0].name, "Sin");
    }

    #[test]
    fn load_from_dir_errors_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mapSolarSystems.jsonl"), "").unwrap();
        // mapStargates.jsonl absent.
        let err = load_from_dir(dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing mapStargates.jsonl"));
    }

    #[test]
    fn load_from_dir_errors_when_type_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mapSolarSystems.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("mapStargates.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("types.jsonl"), "").unwrap();
        // typeDogma.jsonl absent — the four-file build is incomplete.
        let err = load_from_dir(dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing typeDogma.jsonl"));
    }

    #[test]
    fn load_build_treats_missing_type_files_as_incomplete() {
        // A cached build with only the two map files is incomplete now that the
        // type files are required: it must be rejected, not loaded partially.
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        let build = 12345;
        let build_dir = cache.build_dir(build);
        std::fs::create_dir_all(&build_dir).unwrap();
        std::fs::write(build_dir.join("mapSolarSystems.jsonl"), b"").unwrap();
        std::fs::write(build_dir.join("mapStargates.jsonl"), b"").unwrap();
        // types.jsonl / typeDogma.jsonl absent.
        assert!(!cache.build_files_present(build));
        let err = cache.load_build(build).unwrap_err();
        assert!(err.to_string().contains("incomplete"));
        // And metadata pointing at it is treated as absent (re-fetched).
        cache
            .write_metadata(&CacheMetadata {
                build_number: build,
                release_date: None,
                fetched_at: Utc::now(),
            })
            .unwrap();
        assert!(cache.current_metadata().is_none());
    }

    #[test]
    fn extract_files_writes_required_files_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let zip_bytes = make_zip_with_files(&[
            ("mapSolarSystems.jsonl", b"{}\n"),
            ("mapStargates.jsonl", b""),
            ("types.jsonl", b""),
            ("typeDogma.jsonl", b""),
            ("mapRegions.jsonl", b""),
            ("ignored_extra.jsonl", b"junk"),
        ]);
        extract_files(Cursor::new(zip_bytes), dir.path()).unwrap();
        for f in FILES {
            assert!(dir.path().join(f).exists(), "{f} missing");
        }
        // Extras shouldn't be written.
        assert!(!dir.path().join("ignored_extra.jsonl").exists());
        assert!(!dir.path().join("mapRegions.jsonl").exists());
        // No leftover .tmp files (rename completed).
        for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
            assert!(
                !entry.file_name().to_string_lossy().ends_with(".tmp"),
                "leftover tmp"
            );
        }
    }

    #[test]
    fn extract_files_rejects_zip_slip() {
        let dir = tempfile::tempdir().unwrap();
        let zip_bytes = make_zip_with_files_raw(&[
            ("mapSolarSystems.jsonl", b"{}"),
            ("mapStargates.jsonl", b""),
            ("types.jsonl", b""),
            ("typeDogma.jsonl", b""),
            ("../escape.jsonl", b"pwned"),
            ("/abs/escape.jsonl", b"pwned"),
            ("nested/../../escape.jsonl", b"pwned"),
        ]);
        extract_files(Cursor::new(zip_bytes), dir.path()).unwrap();
        for f in FILES {
            assert!(dir.path().join(f).exists(), "{f} missing");
        }
        let parent = dir.path().parent().unwrap();
        assert!(
            !parent.join("escape.jsonl").exists(),
            "ZIP slip wrote escape.jsonl"
        );
    }

    #[test]
    fn is_unsafe_zip_path_flags_traversal_and_absolute() {
        assert!(is_unsafe_zip_path("../escape"));
        assert!(is_unsafe_zip_path("/abs/path"));
        assert!(is_unsafe_zip_path("C:\\evil"));
        assert!(is_unsafe_zip_path("nested/../../escape"));
        assert!(!is_unsafe_zip_path("Latest/mapSolarSystems.jsonl"));
    }

    #[test]
    fn extract_files_fails_on_missing_required() {
        let dir = tempfile::tempdir().unwrap();
        let zip_bytes = make_zip_with_files(&[("mapSolarSystems.jsonl", b"")]);
        let err = extract_files(Cursor::new(zip_bytes), dir.path()).unwrap_err();
        assert!(err.to_string().contains("not found in SDE zip"));
    }

    #[test]
    fn current_metadata_none_when_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        std::fs::write(cache.metadata_path(), b"{ truncated").unwrap();
        assert!(cache.current_metadata().is_none());
    }

    #[test]
    fn load_build_errors_when_files_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        let err = cache.load_build(99).unwrap_err();
        assert!(err.to_string().contains("incomplete"));
    }

    #[test]
    fn current_metadata_none_when_files_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        cache
            .write_metadata(&CacheMetadata {
                build_number: 12345,
                release_date: None,
                fetched_at: Utc::now(),
            })
            .unwrap();
        assert!(cache.current_metadata().is_none());
    }

    #[test]
    fn current_metadata_some_when_complete() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        let build = 12345;
        let build_dir = cache.build_dir(build);
        std::fs::create_dir_all(&build_dir).unwrap();
        for f in FILES {
            std::fs::write(build_dir.join(f), b"").unwrap();
        }
        cache
            .write_metadata(&CacheMetadata {
                build_number: build,
                release_date: None,
                fetched_at: Utc::now(),
            })
            .unwrap();
        let meta = cache.current_metadata().unwrap();
        assert_eq!(meta.build_number, build);
    }

    #[test]
    fn prune_removes_old_builds_keeps_current() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SdeCache::new(dir.path().to_path_buf());
        for b in [100u64, 200, 300] {
            std::fs::create_dir_all(cache.build_dir(b)).unwrap();
        }
        cache.prune_other_builds(200).unwrap();
        assert!(!cache.build_dir(100).exists());
        assert!(cache.build_dir(200).exists());
        assert!(!cache.build_dir(300).exists());
    }

    #[test]
    fn parses_latest_manifest_shape() {
        let line =
            r#"{"_key": "sde", "buildNumber": 3333874, "releaseDate": "2026-05-06T11:43:57Z"}"#;
        let m: LatestManifest = serde_json::from_str(line).unwrap();
        assert_eq!(m.build_number, 3333874);
    }
}

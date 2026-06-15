//! Environment configuration and tracing init.
//!
//! All runtime knobs use the `PALU_*` convention. Every getter has a
//! documented default so the service runs with zero configuration.

use std::time::Duration;

use tracing_subscriber::EnvFilter;

/// Default TCP port. Overridable via `PALU_PORT`.
pub const DEFAULT_PORT: u16 = 5001;
/// Default SDE freshness-check interval. CCP publishes new builds roughly
/// daily, so an hour balances staleness against load on their endpoint.
pub const DEFAULT_SDE_RELOAD_INTERVAL_SECS: u64 = 3600;
/// Default EVE-Scout poll interval. Their signatures churn over minutes, so
/// ten minutes keeps the snapshot fresh without hammering the API.
pub const DEFAULT_EVE_SCOUT_INTERVAL_SECS: u64 = 600;

/// Resolve the listen port from `PALU_PORT`, falling back to
/// [`DEFAULT_PORT`]. An unparseable value falls back to the default with a
/// warning rather than failing startup.
pub fn port() -> u16 {
    parse_env_or("PALU_PORT", DEFAULT_PORT)
}

/// SDE reload interval in seconds (`PALU_SDE_RELOAD_INTERVAL_SECS`).
/// `0` disables the background reload task.
pub fn sde_reload_interval() -> Option<Duration> {
    interval_from_secs(parse_env_or(
        "PALU_SDE_RELOAD_INTERVAL_SECS",
        DEFAULT_SDE_RELOAD_INTERVAL_SECS,
    ))
}

/// EVE-Scout poll interval in seconds (`PALU_EVE_SCOUT_INTERVAL_SECS`).
/// `0` disables the background poller.
pub fn eve_scout_interval() -> Option<Duration> {
    interval_from_secs(parse_env_or(
        "PALU_EVE_SCOUT_INTERVAL_SECS",
        DEFAULT_EVE_SCOUT_INTERVAL_SECS,
    ))
}

/// The application version. CI derives the real version from git tags and
/// passes it in via `PALU_APP_VERSION` (set as an image `ENV` from the Docker
/// `APP_VERSION` build arg). For local, non-Docker runs the env var is unset,
/// so this falls back to the crate version — which is the placeholder `0.0.0`,
/// signalling "unversioned dev build". This is the single source of truth for
/// the version: the `/health` endpoint and the ESI user-agent both read it.
pub fn app_version() -> String {
    std::env::var("PALU_APP_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

/// The git commit the build was cut from (`PALU_GIT_COMMIT_SHA`, set as an
/// image `ENV` from the Docker `GIT_COMMIT_SHA` build arg by CI). `"unknown"`
/// for local builds where CI did not stamp it.
pub fn git_commit() -> String {
    std::env::var("PALU_GIT_COMMIT_SHA").unwrap_or_else(|_| "unknown".to_string())
}

/// `Some(Duration)` for a positive interval, `None` for `0` (disabled).
fn interval_from_secs(secs: u64) -> Option<Duration> {
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Parse an env var into `T`, falling back to `default` when unset or
/// unparseable. A present-but-invalid value is logged.
fn parse_env_or<T>(key: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    match std::env::var(key) {
        Err(_) => default,
        Ok(raw) => raw.parse().unwrap_or_else(|_| {
            tracing::warn!(key, value = %raw, "invalid value, using default");
            default
        }),
    }
}

/// Initialise the tracing subscriber. Honours `RUST_LOG`, defaulting to
/// `info`. Idempotent-ish: a second call is a no-op because the global
/// subscriber is already set (`try_init` swallows the error).
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_or_unset_uses_default() {
        // SAFETY: name unique to this test; no other thread touches it.
        unsafe { std::env::remove_var("PALU_TEST_UNSET") };
        assert_eq!(parse_env_or("PALU_TEST_UNSET", 5001u16), 5001);
    }

    #[test]
    fn parse_env_or_valid_value_parses() {
        unsafe { std::env::set_var("PALU_TEST_VALID", "9090") };
        assert_eq!(parse_env_or("PALU_TEST_VALID", 5001u16), 9090);
    }

    #[test]
    fn parse_env_or_invalid_value_uses_default() {
        unsafe { std::env::set_var("PALU_TEST_INVALID", "not-a-number") };
        assert_eq!(parse_env_or("PALU_TEST_INVALID", 5001u16), 5001);
    }

    #[test]
    fn interval_zero_disables() {
        assert_eq!(interval_from_secs(0), None);
    }

    #[test]
    fn interval_positive_enables() {
        assert_eq!(interval_from_secs(3600), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn app_version_falls_back_to_crate_version() {
        // SAFETY: env access is process-global; these version vars are not
        // touched by other tests.
        unsafe { std::env::remove_var("PALU_APP_VERSION") };
        // Local (unset) → the crate version, which is the 0.0.0 placeholder.
        assert_eq!(app_version(), env!("CARGO_PKG_VERSION"));
        unsafe { std::env::set_var("PALU_APP_VERSION", "1.2.3") };
        assert_eq!(app_version(), "1.2.3");
        unsafe { std::env::remove_var("PALU_APP_VERSION") };
    }

    #[test]
    fn git_commit_defaults_to_unknown() {
        unsafe { std::env::remove_var("PALU_GIT_COMMIT_SHA") };
        assert_eq!(git_commit(), "unknown");
        unsafe { std::env::set_var("PALU_GIT_COMMIT_SHA", "abc1234") };
        assert_eq!(git_commit(), "abc1234");
        unsafe { std::env::remove_var("PALU_GIT_COMMIT_SHA") };
    }
}

//! Environment configuration and tracing init.
//!
//! All runtime knobs use the `GEODESIC_*` convention. Every getter has a
//! documented default so the service runs with zero configuration.

use std::time::Duration;

use tracing_subscriber::EnvFilter;

/// Default TCP port. Overridable via `GEODESIC_PORT`.
pub const DEFAULT_PORT: u16 = 5001;
/// Default SDE freshness-check interval. CCP publishes new builds roughly
/// daily, so an hour balances staleness against load on their endpoint.
pub const DEFAULT_SDE_RELOAD_INTERVAL_SECS: u64 = 3600;
/// Default EVE-Scout poll interval. Their signatures churn over minutes, so
/// ten minutes keeps the snapshot fresh without hammering the API.
pub const DEFAULT_EVE_SCOUT_INTERVAL_SECS: u64 = 600;

/// Resolve the listen port from `GEODESIC_PORT`, falling back to
/// [`DEFAULT_PORT`]. An unparseable value falls back to the default with a
/// warning rather than failing startup.
pub fn port() -> u16 {
    parse_env_or("GEODESIC_PORT", DEFAULT_PORT)
}

/// SDE reload interval in seconds (`GEODESIC_SDE_RELOAD_INTERVAL_SECS`).
/// `0` disables the background reload task.
pub fn sde_reload_interval() -> Option<Duration> {
    interval_from_secs(parse_env_or(
        "GEODESIC_SDE_RELOAD_INTERVAL_SECS",
        DEFAULT_SDE_RELOAD_INTERVAL_SECS,
    ))
}

/// EVE-Scout poll interval in seconds (`GEODESIC_EVE_SCOUT_INTERVAL_SECS`).
/// `0` disables the background poller.
pub fn eve_scout_interval() -> Option<Duration> {
    interval_from_secs(parse_env_or(
        "GEODESIC_EVE_SCOUT_INTERVAL_SECS",
        DEFAULT_EVE_SCOUT_INTERVAL_SECS,
    ))
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
        unsafe { std::env::remove_var("GEODESIC_TEST_UNSET") };
        assert_eq!(parse_env_or("GEODESIC_TEST_UNSET", 5001u16), 5001);
    }

    #[test]
    fn parse_env_or_valid_value_parses() {
        unsafe { std::env::set_var("GEODESIC_TEST_VALID", "9090") };
        assert_eq!(parse_env_or("GEODESIC_TEST_VALID", 5001u16), 9090);
    }

    #[test]
    fn parse_env_or_invalid_value_uses_default() {
        unsafe { std::env::set_var("GEODESIC_TEST_INVALID", "not-a-number") };
        assert_eq!(parse_env_or("GEODESIC_TEST_INVALID", 5001u16), 5001);
    }

    #[test]
    fn interval_zero_disables() {
        assert_eq!(interval_from_secs(0), None);
    }

    #[test]
    fn interval_positive_enables() {
        assert_eq!(interval_from_secs(3600), Some(Duration::from_secs(3600)));
    }
}

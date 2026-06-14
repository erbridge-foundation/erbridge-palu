//! Background pollers: SDE hot-reload and EVE-Scout signature refresh. Both
//! build a new immutable value fully in memory, then atomically swap it into
//! the live `ArcSwap`. Failures are logged and non-fatal — the previous value
//! keeps serving.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::{ArcSwap, ArcSwapOption};
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::eve_scout::{self, EveScoutSnapshot};
use crate::graph::build_graph_data;
use crate::model::GraphData;
use crate::sde::cache::{RefreshOutcome, SdeCache, check_and_refresh};

/// Spawn the SDE hot-reload task. Polls the manifest at `interval`; when a
/// newer build appears it builds the new `GraphData` fully in memory and only
/// then swaps it in (memory transiently doubles during the build — see
/// `AppState::graph`). Network/parse failures are logged and retried next tick.
pub fn spawn_sde_reload(
    graph: Arc<ArcSwap<GraphData>>,
    last_reload_at: Arc<ArcSwapOption<DateTime<Utc>>>,
    cache: Arc<SdeCache>,
    client: reqwest::Client,
    interval: Duration,
) {
    info!(
        interval_secs = interval.as_secs(),
        "spawning SDE hot-reload task"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick fires immediately; skip it so we don't re-check right
        // after the synchronous startup load.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let current_build = graph.load().build_number;
            match check_and_refresh(&client, &cache, current_build).await {
                Ok(RefreshOutcome::UpToDate) => {
                    info!(build = current_build, "SDE cache is current");
                }
                Ok(RefreshOutcome::Updated { data, meta }) => {
                    let new_graph = Arc::new(build_graph_data(data, meta.build_number));
                    info!(
                        build = new_graph.build_number,
                        systems = new_graph.systems.len(),
                        edges = new_graph.gate_graph.edge_count(),
                        "SDE hot-reloaded"
                    );
                    graph.store(new_graph);
                    last_reload_at.store(Some(Arc::new(Utc::now())));
                }
                Err(e) => {
                    warn!(error = %e, "SDE freshness check failed, will retry");
                }
            }
        }
    });
}

/// Spawn the EVE-Scout poller. Fetches signatures at `interval` and swaps in a
/// fresh snapshot. The first tick fires immediately so the snapshot is
/// populated soon after startup. A failed poll is logged and the last snapshot
/// is retained.
pub fn spawn_eve_scout_poll(
    snapshot: Arc<ArcSwap<EveScoutSnapshot>>,
    client: reqwest::Client,
    interval: Duration,
) {
    info!(
        interval_secs = interval.as_secs(),
        "spawning EVE-Scout poll task"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match eve_scout::fetch_snapshot(&client).await {
                Ok(snap) => {
                    info!(sig_count = snap.sig_count(), "EVE-Scout snapshot refreshed");
                    snapshot.store(Arc::new(snap));
                }
                Err(e) => {
                    warn!(error = %e, "EVE-Scout poll failed, keeping last snapshot");
                }
            }
        }
    });
}

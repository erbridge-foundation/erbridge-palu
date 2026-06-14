//! Shared application state. Two `ArcSwap`s hold the hot-reloadable SDE graph
//! and the EVE-Scout snapshot; handlers `.load()` a per-request snapshot
//! lock-free.

use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};
use chrono::{DateTime, Utc};

use crate::eve_scout::EveScoutSnapshot;
use crate::model::GraphData;

#[derive(Clone)]
pub struct AppState {
    /// Live SDE graph. Behind `ArcSwap` so the hot-reload task can publish a
    /// new `Arc<GraphData>` without blocking handlers. During a swap memory
    /// transiently doubles: the old graph still serves in-flight requests
    /// while the new one is fully built before `store`.
    pub graph: Arc<ArcSwap<GraphData>>,
    /// Wall-clock time of the last successful SDE reload swap (not a no-op
    /// freshness check). `None` until the first real swap.
    pub last_reload_at: Arc<ArcSwapOption<DateTime<Utc>>>,
    /// EVE-Scout signatures snapshot, refreshed by its own background poller.
    pub eve_scout: Arc<ArcSwap<EveScoutSnapshot>>,
}

impl AppState {
    /// Wrap a freshly-loaded `GraphData`. The reload timestamp starts `None`
    /// and the EVE-Scout snapshot starts empty until the first poll.
    pub fn new(graph: Arc<GraphData>) -> Self {
        Self {
            graph: Arc::new(ArcSwap::from(graph)),
            last_reload_at: Arc::new(ArcSwapOption::const_empty()),
            eve_scout: Arc::new(ArcSwap::from_pointee(EveScoutSnapshot::default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph_data;
    use crate::model::RawSdeData;

    fn empty_graph(build: u64) -> Arc<GraphData> {
        Arc::new(build_graph_data(
            RawSdeData {
                systems: vec![],
                gate_pairs: vec![],
            },
            build,
        ))
    }

    #[test]
    fn new_starts_with_no_reload_and_empty_scout() {
        let state = AppState::new(empty_graph(7));
        assert_eq!(state.graph.load().build_number, 7);
        assert!(state.last_reload_at.load_full().is_none());
        assert_eq!(state.eve_scout.load().sig_count(), 0);
    }

    #[test]
    fn graph_swap_is_visible() {
        let state = AppState::new(empty_graph(1));
        state.graph.store(empty_graph(2));
        assert_eq!(state.graph.load().build_number, 2);
    }
}

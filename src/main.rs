//! Service entry point: load the SDE, build the graph, spawn the background
//! pollers, and serve the axum app.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tracing::info;

use erbridge_palu::app_state::AppState;
use erbridge_palu::config;
use erbridge_palu::graph::build_graph_data;
use erbridge_palu::sde::cache::{
    SdeCache, ensure_cache, load_from_dir, resolve_cache_dir, resolve_sde_dir,
};
use erbridge_palu::{build_router, tasks};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::init_tracing();

    let cache_dir = resolve_cache_dir().context("resolving SDE cache dir")?;
    let cache = Arc::new(SdeCache::new(cache_dir));
    let client = reqwest::Client::builder()
        .user_agent(format!("erbridge-palu/{}", config::app_version()))
        .build()
        .context("building HTTP client")?;

    // PALU_SDE_DIR short-circuits the download path: load the two JSONL
    // files directly (offline). When set, the SDE hot-reload poller is skipped
    // since there is no manifest to compare against.
    let sde_dir = resolve_sde_dir();
    let (raw, meta) = match &sde_dir {
        Some(dir) => load_from_dir(dir).context("loading SDE from PALU_SDE_DIR")?,
        None => {
            info!(cache_dir = %cache.root.display(), "loading SDE");
            // Initial load is fatal — we can't serve without a graph.
            ensure_cache(&client, &cache)
                .await
                .context("ensuring SDE cache")?
        }
    };
    let graph = Arc::new(build_graph_data(raw, meta.build_number));
    info!(
        build = graph.build_number,
        systems = graph.systems.len(),
        edges = graph.gate_graph.edge_count(),
        "SDE loaded"
    );

    let state = AppState::new(graph);

    match (sde_dir.is_some(), config::sde_reload_interval()) {
        (true, _) => info!("SDE hot-reload disabled (PALU_SDE_DIR set)"),
        (false, Some(interval)) => tasks::spawn_sde_reload(
            state.graph.clone(),
            state.last_reload_at.clone(),
            cache.clone(),
            client.clone(),
            interval,
        ),
        (false, None) => info!("SDE hot-reload disabled (interval=0)"),
    }

    if let Some(interval) = config::eve_scout_interval() {
        tasks::spawn_eve_scout_poll(state.eve_scout.clone(), client.clone(), interval);
    } else {
        info!("EVE-Scout poller disabled (interval=0)");
    }

    let app = build_router(state);

    let port = config::port();
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;
    info!(%addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")
}

/// Resolve when SIGINT (Ctrl-C) or SIGTERM (container stop) arrives, so axum can
/// stop accepting and drain in-flight requests before the process exits.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c().await.expect("install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received, draining in-flight requests");
}

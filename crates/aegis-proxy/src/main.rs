use std::{path::PathBuf, sync::Arc, time::Duration};

use aegis_core::{AddressSets, Blocklist, WasmEngine};
use aegis_proxy::{
    config::Config,
    feeds::SourceRegistry,
    handler::{metrics_handler, rpc_handler, AppState, MAX_BODY_BYTES},
    lut::LutResolver,
    metrics::Metrics,
};
use axum::{routing::any, Router};
use tower_http::{catch_panic::CatchPanicLayer, cors::CorsLayer};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().json().init();

    let cfg = Config::load().unwrap_or_else(|e| panic!("invalid configuration: {e}"));
    info!(listen_addr = %cfg.listen_addr, downstream = %cfg.downstream_url, "Aegis starting");

    let blocklist = Blocklist::new(AddressSets::default());
    let sources = Arc::new(
        SourceRegistry::new(
            blocklist.clone(),
            PathBuf::from(&cfg.blocklist_path),
            cfg.threat_feeds.clone(),
        )
        .expect("failed to load blocklist"),
    );

    let metrics = Metrics::new().expect("failed to init metrics");
    metrics.blocklist_size.set(blocklist.len() as f64);
    metrics.downstream_up.set(1.0);

    let wasm_engine = cfg
        .wasm_plugin_path
        .as_deref()
        .map(|p| WasmEngine::load(std::path::Path::new(p)).expect("failed to load wasm plugin"));
    if let Some(ref path) = cfg.wasm_plugin_path {
        info!(plugin = %path, "Wasm rule engine loaded");
    }

    let downstream = reqwest::Client::new();
    let state = Arc::new(AppState {
        blocklist,
        luts: Arc::new(LutResolver::new(
            downstream.clone(),
            cfg.downstream_url.clone(),
        )),
        downstream,
        downstream_url: cfg.downstream_url.clone(),
        metrics,
        wasm_engine,
    });

    if !sources.feeds().is_empty() {
        let (sources, state) = (sources.clone(), state.clone());
        let interval = Duration::from_secs(cfg.threat_feed_interval_secs);
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                let results = sources.refresh_feeds(&client).await;
                for (feed, result) in sources.feeds().iter().zip(results) {
                    let outcome = match result {
                        Ok(n) => {
                            info!(feed = %feed.name, entries = n, "threat feed refreshed");
                            "ok"
                        }
                        Err(e) => {
                            warn!(feed = %feed.name, "threat feed refresh failed, keeping last good copy: {e}");
                            "error"
                        }
                    };
                    state
                        .metrics
                        .feed_refresh_total
                        .with_label_values(&[&feed.name, outcome])
                        .inc();
                }
                state
                    .metrics
                    .blocklist_size
                    .set(state.blocklist.len() as f64);
            }
        });
    }

    // SIGHUP: reload the local blocklist file (feeds keep their cached contents) and the Wasm plugin.
    let (sighup_sources, sighup_state) = (sources.clone(), state.clone());
    let sighup_wasm_path = cfg.wasm_plugin_path.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sig = signal(SignalKind::hangup()).expect("failed to register SIGHUP");
        loop {
            sig.recv().await;
            match sighup_sources.reload_file() {
                Ok(n) => {
                    sighup_state.metrics.blocklist_size.set(n as f64);
                    info!("blocklist reloaded: {n} entries");
                }
                Err(e) => error!("blocklist reload failed, keeping previous list: {e}"),
            }
            if let (Some(engine), Some(path)) = (&sighup_state.wasm_engine, &sighup_wasm_path) {
                match engine.reload(std::path::Path::new(path)) {
                    Ok(()) => info!(plugin = %path, "wasm plugin reloaded"),
                    Err(e) => error!("wasm reload failed: {e}"),
                }
            }
        }
    });

    let app = Router::new()
        .route("/metrics", axum::routing::get(metrics_handler))
        .fallback(any(rpc_handler))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        // RPC endpoints are called cross-origin by wallets and dapps; without CORS a
        // browser-context request is rejected before it reaches the firewall.
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr)
        .await
        .expect("failed to bind");
    info!("listening on {}", cfg.listen_addr);
    axum::serve(listener, app).await.expect("server error");
}

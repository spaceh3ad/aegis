mod config;
mod handler;
mod metrics;

use std::{path::PathBuf, sync::Arc};

use aegis_core::{Blocklist, WasmEngine};
use axum::{routing::any, Router};
use handler::{rpc_handler, AppState, metrics_handler};
use metrics::Metrics;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().json().init();

    let cfg = config::Config::load();
    info!(listen_addr = %cfg.listen_addr, downstream = %cfg.downstream_url, "Aegis-SVM starting");

    let blocklist_path = PathBuf::from(&cfg.blocklist_path);
    let blocklist = Blocklist::load_from_file(&blocklist_path)
        .expect("failed to load blocklist");

    let metrics = Metrics::new().expect("failed to init metrics");
    metrics.blocklist_size.set(blocklist.len() as f64);
    metrics.downstream_up.set(1.0);

    let wasm_engine = cfg.wasm_plugin_path.as_deref().map(|p| {
        WasmEngine::load(std::path::Path::new(p))
            .expect("failed to load wasm plugin")
    });
    if let Some(ref path) = cfg.wasm_plugin_path {
        info!(plugin = %path, "Wasm rule engine loaded");
    }

    let state = Arc::new(AppState {
        blocklist: blocklist.clone(),
        downstream: reqwest::Client::new(),
        downstream_url: cfg.downstream_url.clone(),
        metrics,
        wasm_engine,
    });

    // SIGHUP → reload blocklist and Wasm plugin
    let sighup_state = state.clone();
    let sighup_path = blocklist_path.clone();
    let sighup_wasm_path = cfg.wasm_plugin_path.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sig = signal(SignalKind::hangup()).expect("failed to register SIGHUP");
        loop {
            sig.recv().await;
            match sighup_state.blocklist.reload(&sighup_path) {
                Ok(()) => {
                    sighup_state.metrics.blocklist_size.set(sighup_state.blocklist.len() as f64);
                    info!("blocklist reloaded: {} entries", sighup_state.blocklist.len());
                }
                Err(e) => tracing::error!("blocklist reload failed: {e}"),
            }
            if let (Some(engine), Some(path)) = (&sighup_state.wasm_engine, &sighup_wasm_path) {
                match engine.reload(std::path::Path::new(path)) {
                    Ok(()) => info!(plugin = %path, "wasm plugin reloaded"),
                    Err(e) => tracing::error!("wasm reload failed: {e}"),
                }
            }
        }
    });

    let app = Router::new()
        .route("/metrics", axum::routing::get(metrics_handler))
        .fallback(any(rpc_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr)
        .await
        .expect("failed to bind");
    info!("listening on {}", cfg.listen_addr);
    axum::serve(listener, app).await.expect("server error");
}

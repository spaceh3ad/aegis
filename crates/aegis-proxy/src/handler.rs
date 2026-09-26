use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use aegis_core::{parse_transaction, Blocklist, RejectionResponse, Verdict, WasmEngine};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::metrics::Metrics;

static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_trace_id() -> String {
    format!("{:#010x}", TRACE_COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[derive(Clone)]
pub struct AppState {
    pub blocklist: Blocklist,
    pub downstream: reqwest::Client,
    pub downstream_url: String,
    pub metrics: Metrics,
    /// Optional Stage 3: Wasm dynamic rule engine.
    pub wasm_engine: Option<WasmEngine>,
}

pub async fn rpc_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(rpc) = serde_json::from_slice::<Value>(&body) else {
        return forward_raw(&state, &headers, body).await;
    };

    let method = rpc.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = rpc.get("id").cloned().unwrap_or(Value::Null);

    if method == "sendTransaction" || method == "sendVersionedTransaction" {
        let start = Instant::now();

        let b64 = rpc
            .get("params")
            .and_then(|p| p.get(0))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let tx = match parse_transaction(b64) {
            Err(_) => {
                state.metrics.transactions_total.with_label_values(&["allow"]).inc();
                return forward_raw(&state, &headers, body).await;
            }
            Ok(tx) => tx,
        };

        let static_verdict = state.blocklist.check(&tx);

        let elapsed_us = start.elapsed().as_micros() as f64;
        state.metrics.inspection_duration.observe(elapsed_us);

        // Stage 3: Wasm dynamic rules (runs only when a plugin is configured)
        let final_verdict = match static_verdict {
            Some(v) => Some(v),
            None => {
                if let Some(engine) = &state.wasm_engine {
                    engine.evaluate(&tx, 0, 0)
                } else {
                    None
                }
            }
        };

        match final_verdict {
            Some(Verdict::Block { rule_id, code, message }) => {
                state
                    .metrics
                    .transactions_total
                    .with_label_values(&["block"])
                    .inc();
                state
                    .metrics
                    .transactions_blocked_total
                    .with_label_values(&[&rule_id])
                    .inc();

                let rejection = RejectionResponse::new(id, code, message, &rule_id, next_trace_id());
                let body = serde_json::to_vec(&rejection).unwrap_or_default();
                return (StatusCode::OK, [("content-type", "application/json")], body)
                    .into_response();
            }
            Some(Verdict::Drop) => {
                state.metrics.transactions_total.with_label_values(&["drop"]).inc();
                return StatusCode::NO_CONTENT.into_response();
            }
            _ => {
                state.metrics.transactions_total.with_label_values(&["allow"]).inc();
            }
        }
    }

    forward_raw(&state, &headers, body).await
}

pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.render()
}

async fn forward_raw(state: &AppState, headers: &HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    match state
        .downstream
        .post(&state.downstream_url)
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let resp_body = resp.bytes().await.unwrap_or_default();
            (status, resp_body).into_response()
        }
        Err(_) => {
            state.metrics.downstream_up.set(0.0);
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use aegis_core::{
    parse_evm_transaction, parse_transaction, parse_transaction_base58, Blocklist,
    RejectionResponse, RuleInput, Verdict, WasmEngine,
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::{
    lut::{LutOutcome, LutResolver},
    metrics::Metrics,
};

static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Largest JSON-RPC body accepted from a client, and the largest batch array we will screen.
/// Real RPC traffic is small; the caps stop one request from forcing unbounded work/memory.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_LEN: usize = 100;

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
    pub luts: Arc<LutResolver>,
}

pub async fn rpc_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.len() > MAX_BODY_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Array(batch)) => handle_batch(&state, &headers, body, batch).await,
        Ok(req) => match screen(&state, &req).await {
            Some(Verdict::Block {
                rule_id,
                code,
                message,
            }) => json_response(&rejection(&req, code, message, &rule_id)),
            Some(Verdict::Drop) => StatusCode::NO_CONTENT.into_response(),
            _ => forward_raw(&state, &headers, body).await,
        },
        Err(_) => forward_raw(&state, &headers, body).await,
    }
}

pub async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.metrics.render()
}

/// Runs the static + Wasm pipeline on transaction-submitting methods and records metrics.
/// Returns `None` for every other method. Unparseable transactions are forwarded: the
/// downstream node rejects them with the same deserializer, so nothing slips through.
async fn screen(state: &AppState, req: &Value) -> Option<Verdict> {
    let method = req.get("method").and_then(Value::as_str)?;
    let param0 = req
        .get("params")
        .and_then(|p| p.get(0))
        .and_then(Value::as_str)
        .unwrap_or("");
    let start = Instant::now();

    let (static_verdict, rule_input) = match method {
        "sendTransaction" | "sendVersionedTransaction" => {
            let encoding = req
                .get("params")
                .and_then(|p| p.get(1))
                .and_then(|o| o.get("encoding"))
                .and_then(Value::as_str)
                .unwrap_or("base58");
            let parsed = if encoding == "base64" {
                parse_transaction(param0)
            } else {
                parse_transaction_base58(param0)
            };
            match parsed {
                Ok(mut tx) => {
                    resolve_luts(state, &mut tx).await;
                    (state.blocklist.check(&tx), RuleInput::from(&tx))
                }
                Err(_) => return count(state, Verdict::Allow),
            }
        }
        "eth_sendRawTransaction" => match parse_evm_transaction(param0) {
            Ok(tx) => (state.blocklist.check_evm(&tx), RuleInput::from(&tx)),
            Err(_) => return count(state, Verdict::Allow),
        },
        _ => return None,
    };
    state
        .metrics
        .inspection_duration
        .observe(start.elapsed().as_micros() as f64);

    let verdict = static_verdict
        .or_else(|| {
            state
                .wasm_engine
                .as_ref()
                .and_then(|e| e.evaluate(&rule_input))
        })
        .unwrap_or(Verdict::Allow);
    log_verdict(&rule_input, &verdict);
    count(state, verdict)
}

/// One human-readable line per screened transaction, for the side-by-side demo terminal.
fn log_verdict(input: &aegis_core::RuleInput, verdict: &Verdict) {
    let target = input
        .programs
        .first()
        .cloned()
        .unwrap_or_else(|| "-".to_string());
    let signer = input
        .signers
        .first()
        .cloned()
        .unwrap_or_else(|| "-".to_string());
    let what = match input.chain {
        "evm" => format!("selector={}", input.selector.as_deref().unwrap_or("-")),
        _ => format!("cu={} fee={}", input.cu_requested, input.priority_fee),
    };
    match verdict {
        Verdict::Block { rule_id, .. } => tracing::warn!(
            rule = %rule_id,
            tx = %input.chain,
            target = %target,
            signer = %signer,
            %what,
            "BLOCKED"
        ),
        Verdict::Drop => {
            tracing::warn!(tx = %input.chain, target = %target, signer = %signer, %what, "DROPPED")
        }
        Verdict::Allow => {
            tracing::info!(tx = %input.chain, target = %target, signer = %signer, %what, "allowed")
        }
    }
}

// Resolution failures fall back to static keys only. Each failure mode is one the node
// also rejects: a missing or non-LUT table account, an index past the table's end (we
// read at `processed`, the freshest view), or an unreachable node that can't take the tx.
async fn resolve_luts(state: &AppState, tx: &mut aegis_core::InspectedTx) {
    let label = match state.luts.resolve(tx).await {
        Ok(LutOutcome::None) => return,
        Ok(LutOutcome::Cached) => "cached",
        Ok(LutOutcome::Fetched) => "fetched",
        Err(e) => {
            tracing::warn!("lookup table resolution failed, screening static keys only: {e}");
            "error"
        }
    };
    state
        .metrics
        .lut_resolve_total
        .with_label_values(&[label])
        .inc();
}

fn count(state: &AppState, verdict: Verdict) -> Option<Verdict> {
    let label = match &verdict {
        Verdict::Allow => "allow",
        Verdict::Drop => "drop",
        Verdict::Block { rule_id, .. } => {
            state
                .metrics
                .transactions_blocked_total
                .with_label_values(&[rule_id])
                .inc();
            "block"
        }
    };
    state
        .metrics
        .transactions_total
        .with_label_values(&[label])
        .inc();
    Some(verdict)
}

/// Blocked entries are answered locally, dropped entries get no response, and the rest are
/// forwarded as a smaller batch. JSON-RPC 2.0 matches batch responses by id, so the merged
/// array need not preserve request order.
async fn handle_batch(
    state: &AppState,
    headers: &HeaderMap,
    body: Bytes,
    batch: Vec<Value>,
) -> Response {
    if batch.len() > MAX_BATCH_LEN {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let total = batch.len();
    let mut answered = Vec::new();
    let mut passthrough = Vec::with_capacity(total);
    for req in batch {
        match screen(state, &req).await {
            Some(Verdict::Block {
                rule_id,
                code,
                message,
            }) => {
                answered.push(
                    serde_json::to_value(rejection(&req, code, message, &rule_id))
                        .unwrap_or_default(),
                );
            }
            Some(Verdict::Drop) => {}
            _ => passthrough.push(req),
        }
    }

    if passthrough.len() == total {
        return forward_raw(state, headers, body).await;
    }
    if !passthrough.is_empty() {
        let sub = Bytes::from(serde_json::to_vec(&Value::Array(passthrough)).unwrap_or_default());
        match forward_json(state, sub).await {
            Some(Value::Array(items)) => answered.extend(items),
            Some(single) => answered.push(single),
            None => return StatusCode::BAD_GATEWAY.into_response(),
        }
    }
    if answered.is_empty() {
        return StatusCode::NO_CONTENT.into_response();
    }
    json_response(&Value::Array(answered))
}

fn rejection(req: &Value, code: i32, message: String, rule_id: &str) -> RejectionResponse {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    RejectionResponse::new(id, code, message, rule_id, next_trace_id())
}

fn json_response<T: serde::Serialize>(value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_default();
    (StatusCode::OK, [("content-type", "application/json")], body).into_response()
}

async fn forward_json(state: &AppState, body: Bytes) -> Option<Value> {
    let resp = state
        .downstream
        .post(&state.downstream_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await;
    match resp {
        Ok(r) => {
            state.metrics.downstream_up.set(1.0);
            r.json::<Value>().await.ok()
        }
        Err(_) => {
            state.metrics.downstream_up.set(0.0);
            None
        }
    }
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
            state.metrics.downstream_up.set(1.0);
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

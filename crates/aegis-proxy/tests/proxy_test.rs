use std::{io::Write, path::PathBuf, sync::Arc};

use aegis_core::Blocklist;
use aegis_proxy::{
    handler::{rpc_handler, metrics_handler, AppState},
    metrics::Metrics,
};
use axum::{routing::any, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Client;
use serde_json::{json, Value};
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

async fn start_proxy(blocklist_path: PathBuf, downstream_url: String) -> String {
    let blocklist = Blocklist::load_from_file(&blocklist_path).unwrap();
    let metrics = Metrics::new().unwrap();
    metrics.blocklist_size.set(blocklist.len() as f64);
    metrics.downstream_up.set(1.0);

    let state = Arc::new(AppState {
        blocklist,
        downstream: Client::new(),
        downstream_url,
        metrics,
        wasm_engine: None,
    });

    let app = Router::new()
        .route("/metrics", axum::routing::get(metrics_handler))
        .fallback(any(rpc_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

fn write_blocklist(keys: &[&str]) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    for k in keys {
        writeln!(f, "{}", k).unwrap();
    }
    f.flush().unwrap();
    f
}

fn make_tx_with_key(pubkey_b58: &str) -> String {
    use solana_sdk::{
        message::{v0, MessageHeader, VersionedMessage},
        pubkey::Pubkey,
        transaction::VersionedTransaction,
    };
    use std::str::FromStr;

    let key = Pubkey::from_str(pubkey_b58).unwrap();
    let msg = v0::Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 0,
        },
        account_keys: vec![key],
        recent_blockhash: solana_sdk::hash::Hash::default(),
        instructions: vec![],
        address_table_lookups: vec![],
    };
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };
    STANDARD.encode(bincode::serialize(&tx).unwrap())
}

fn make_clean_tx() -> String {
    make_tx_with_key(&solana_sdk::pubkey::Pubkey::new_unique().to_string())
}

fn rpc_body(method: &str, params: Value, id: u64) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

#[tokio::test]
async fn test_blocked_tx_returns_error() {
    let blocked_key = "So11111111111111111111111111111111111111112";
    let tmp = write_blocklist(&[blocked_key]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), "http://127.0.0.1:19999".into()).await;

    let b64 = make_tx_with_key(blocked_key);
    let body = rpc_body("sendTransaction", json!([b64, {"encoding": "base64"}]), 1);

    let resp = Client::new()
        .post(&proxy_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["error"]["code"], -32001);
    assert_eq!(json["error"]["data"]["rule_id"], "ofac-sdn");
}

#[tokio::test]
async fn test_clean_tx_forwards() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","result":"5jSig","id":2})),
        )
        .mount(&mock)
        .await;

    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let body = rpc_body("sendTransaction", json!([make_clean_tx(), {"encoding":"base64"}]), 2);
    let resp = Client::new()
        .post(&proxy_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert!(json.get("result").is_some());
}

#[tokio::test]
async fn test_passthrough_method() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc":"2.0","result":1_000_000,"id":3})),
        )
        .mount(&mock)
        .await;

    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let body = rpc_body("getBalance", json!(["11111111111111111111111111111111"]), 3);
    let resp = Client::new()
        .post(&proxy_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["result"], 1_000_000);
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), "http://127.0.0.1:19999".into()).await;

    let resp = Client::new().get(format!("{proxy_url}/metrics")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("aegis_blocklist_size"));
    assert!(text.contains("aegis_downstream_up"));
}

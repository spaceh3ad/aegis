use std::{io::Write, path::PathBuf, sync::Arc};

use aegis_core::{AddressSets, Blocklist, WasmEngine};
use aegis_proxy::{
    config::FeedSpec,
    feeds::SourceRegistry,
    handler::{metrics_handler, rpc_handler, AppState},
    metrics::Metrics,
};
use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use axum::{routing::any, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::Client;
use serde_json::{json, Value};
use solana_sdk::{
    instruction::CompiledInstruction,
    message::{v0, MessageHeader, VersionedMessage},
    pubkey::Pubkey,
    transaction::VersionedTransaction,
};
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

const DEAD_DOWNSTREAM: &str = "http://127.0.0.1:19999";

async fn serve(blocklist: Blocklist, downstream_url: String, wasm_engine: Option<WasmEngine>) -> String {
    let metrics = Metrics::new().unwrap();
    metrics.blocklist_size.set(blocklist.len() as f64);
    metrics.downstream_up.set(1.0);

    let state = Arc::new(AppState {
        blocklist,
        downstream: Client::new(),
        downstream_url,
        metrics,
        wasm_engine,
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

async fn start_proxy(blocklist_path: PathBuf, downstream_url: String) -> String {
    serve(Blocklist::load_from_file(&blocklist_path).unwrap(), downstream_url, None).await
}

fn write_blocklist(keys: &[&str]) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    for k in keys {
        writeln!(f, "{}", k).unwrap();
    }
    f.flush().unwrap();
    f
}

fn solana_tx_bytes(keys: Vec<Pubkey>, instructions: Vec<CompiledInstruction>, ro_unsigned: u8) -> Vec<u8> {
    let msg = v0::Message {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: ro_unsigned,
        },
        account_keys: keys,
        recent_blockhash: solana_sdk::hash::Hash::default(),
        instructions,
        address_table_lookups: vec![],
    };
    let tx = VersionedTransaction {
        signatures: vec![solana_sdk::signature::Signature::default()],
        message: VersionedMessage::V0(msg),
    };
    bincode::serialize(&tx).unwrap()
}

fn make_tx_with_key(pubkey_b58: &str) -> String {
    use std::str::FromStr;
    STANDARD.encode(solana_tx_bytes(vec![Pubkey::from_str(pubkey_b58).unwrap()], vec![], 0))
}

fn make_clean_tx() -> String {
    make_tx_with_key(&Pubkey::new_unique().to_string())
}

fn raw_evm_tx(signer: &PrivateKeySigner, to: Address) -> String {
    let tx = TxEip1559 {
        chain_id: 8453,
        nonce: 0,
        gas_limit: 21_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000,
        to: TxKind::Call(to),
        value: U256::from(1u64),
        access_list: Default::default(),
        input: Bytes::new(),
    };
    let sig = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
    let env: TxEnvelope = tx.into_signed(sig).into();
    format!("0x{}", hex::encode(env.encoded_2718()))
}

fn rpc_body(method: &str, params: Value, id: u64) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

async fn post(url: &str, body: String) -> reqwest::Response {
    Client::new()
        .post(url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap()
}

async fn ok_downstream(result: Value) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","result":result,"id":1})))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn test_blocklisted_solana_tx_is_rejected_with_source() {
    let blocked_key = "So11111111111111111111111111111111111111112";
    let tmp = write_blocklist(&[blocked_key]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let body = rpc_body("sendTransaction", json!([make_tx_with_key(blocked_key), {"encoding": "base64"}]), 1);
    let resp = post(&proxy_url, body).await;

    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["error"]["code"], -32001);
    assert_eq!(json["error"]["data"]["rule_id"], "blocklist:file");
    assert_eq!(json["id"], 1);
}

#[tokio::test]
async fn test_default_base58_encoding_is_screened() {
    let blocked = Pubkey::new_unique();
    let tmp = write_blocklist(&[&blocked.to_string()]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let b58 = bs58_encode(&solana_tx_bytes(vec![blocked], vec![], 0));
    let resp = post(&proxy_url, rpc_body("sendTransaction", json!([b58]), 4)).await;
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["error"]["code"], -32001, "no encoding option means base58 and must still be screened");
}

fn bs58_encode(bytes: &[u8]) -> String {
    bs58::encode(bytes).into_string()
}

#[tokio::test]
async fn test_clean_tx_forwards() {
    let mock = ok_downstream(json!("5jSig")).await;
    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let body = rpc_body("sendTransaction", json!([make_clean_tx(), {"encoding":"base64"}]), 2);
    let resp = post(&proxy_url, body).await;

    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert!(json.get("result").is_some());
}

#[tokio::test]
async fn test_passthrough_method() {
    let mock = ok_downstream(json!(1_000_000)).await;
    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let resp = post(&proxy_url, rpc_body("getBalance", json!(["11111111111111111111111111111111"]), 3)).await;
    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["result"], 1_000_000);
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let tmp = write_blocklist(&[]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let resp = Client::new().get(format!("{proxy_url}/metrics")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("aegis_blocklist_size"));
    assert!(text.contains("aegis_downstream_up"));
}

#[tokio::test]
async fn test_evm_sender_on_blocklist_is_rejected() {
    let signer = PrivateKeySigner::random();
    let tmp = write_blocklist(&[&format!("{:#x}", signer.address())]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let raw = raw_evm_tx(&signer, Address::repeat_byte(0x11));
    let resp = post(&proxy_url, rpc_body("eth_sendRawTransaction", json!([raw]), 9)).await;
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json["error"]["code"], -32001);
    assert_eq!(json["error"]["data"]["rule_id"], "blocklist:file");
    assert_eq!(json["id"], 9);
}

#[tokio::test]
async fn test_evm_call_to_blocklisted_contract_is_rejected() {
    let target = Address::repeat_byte(0x42);
    let tmp = write_blocklist(&[&format!("{target:#x}")]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let raw = raw_evm_tx(&PrivateKeySigner::random(), target);
    let json: Value = post(&proxy_url, rpc_body("eth_sendRawTransaction", json!([raw]), 1)).await.json().await.unwrap();
    assert_eq!(json["error"]["code"], -32001);
}

#[tokio::test]
async fn test_clean_evm_tx_forwards() {
    let mock = ok_downstream(json!("0xhash")).await;
    let tmp = write_blocklist(&["0x0000000000000000000000000000000000000001"]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let raw = raw_evm_tx(&PrivateKeySigner::random(), Address::repeat_byte(0x11));
    let json: Value = post(&proxy_url, rpc_body("eth_sendRawTransaction", json!([raw]), 1)).await.json().await.unwrap();
    assert_eq!(json["result"], "0xhash");
}

#[tokio::test]
async fn test_batch_blocked_entries_answered_locally_rest_forwarded() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"jsonrpc":"2.0","result":"forwarded","id":2}
        ])))
        .expect(1)
        .mount(&mock)
        .await;

    let blocked = Pubkey::new_unique();
    let tmp = write_blocklist(&[&blocked.to_string()]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), mock.uri()).await;

    let batch = json!([
        {"jsonrpc":"2.0","id":1,"method":"sendTransaction",
         "params":[STANDARD.encode(solana_tx_bytes(vec![blocked], vec![], 0)), {"encoding":"base64"}]},
        {"jsonrpc":"2.0","id":2,"method":"getSlot","params":[]}
    ]);
    let json: Value = post(&proxy_url, batch.to_string()).await.json().await.unwrap();
    let items = json.as_array().expect("batch response is an array");
    assert_eq!(items.len(), 2);

    let by_id = |id: u64| items.iter().find(|i| i["id"] == id).unwrap();
    assert_eq!(by_id(1)["error"]["code"], -32001);
    assert_eq!(by_id(2)["result"], "forwarded");

    let forwarded: Value = serde_json::from_slice(&mock.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(forwarded.as_array().unwrap().len(), 1, "blocked tx must not reach downstream");
    assert_eq!(forwarded[0]["method"], "getSlot");
}

#[tokio::test]
async fn test_batch_fully_blocked_never_touches_downstream() {
    let blocked = Pubkey::new_unique();
    let tmp = write_blocklist(&[&blocked.to_string()]);
    let proxy_url = start_proxy(tmp.path().to_path_buf(), DEAD_DOWNSTREAM.into()).await;

    let tx = STANDARD.encode(solana_tx_bytes(vec![blocked], vec![], 0));
    let batch = json!([
        {"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":[tx, {"encoding":"base64"}]},
        {"jsonrpc":"2.0","id":2,"method":"sendTransaction","params":[tx, {"encoding":"base64"}]}
    ]);
    let resp = post(&proxy_url, batch.to_string()).await;
    assert_eq!(resp.status(), 200);
    let json: Value = resp.json().await.unwrap();
    assert_eq!(json.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_threat_feed_hot_loads_and_attributes_source() {
    let evm_addr = Address::repeat_byte(0x77);
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("# seal feed\n{evm_addr:#x}\n")))
        .mount(&feed_server)
        .await;

    let tmp = write_blocklist(&[]);
    let blocklist = Blocklist::new(AddressSets::default());
    let registry = SourceRegistry::new(
        blocklist.clone(),
        tmp.path().to_path_buf(),
        vec![FeedSpec { name: "seal".into(), url: feed_server.uri() }],
    )
    .unwrap();
    let proxy_url = serve(blocklist.clone(), DEAD_DOWNSTREAM.into(), None).await;
    assert_eq!(blocklist.len(), 0);

    let results = registry.refresh_feeds(&Client::new()).await;
    assert_eq!(results.len(), 1);
    assert_eq!(*results[0].as_ref().unwrap(), 1);

    let raw = raw_evm_tx(&PrivateKeySigner::random(), evm_addr);
    let json: Value = post(&proxy_url, rpc_body("eth_sendRawTransaction", json!([raw]), 1)).await.json().await.unwrap();
    assert_eq!(json["error"]["data"]["rule_id"], "blocklist:seal");
}

#[tokio::test]
async fn test_failing_feed_keeps_last_good_copy() {
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("0x0000000000000000000000000000000000000abc\n"))
        .up_to_n_times(1)
        .mount(&feed_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&feed_server)
        .await;

    let tmp = write_blocklist(&[&Pubkey::new_unique().to_string()]);
    let blocklist = Blocklist::new(AddressSets::default());
    let registry = SourceRegistry::new(
        blocklist.clone(),
        tmp.path().to_path_buf(),
        vec![FeedSpec { name: "trm".into(), url: feed_server.uri() }],
    )
    .unwrap();
    assert_eq!(blocklist.len(), 1);

    assert!(registry.refresh_feeds(&Client::new()).await[0].is_ok());
    assert_eq!(blocklist.len(), 2);

    assert!(registry.refresh_feeds(&Client::new()).await[0].is_err());
    assert_eq!(blocklist.len(), 2, "503 must not drop the feed's entries");

    assert_eq!(registry.reload_file().unwrap(), 2, "file reload keeps cached feed entries");
}

#[tokio::test]
async fn test_malformed_feed_is_rejected_without_publishing() {
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not-an-address!!\n"))
        .mount(&feed_server)
        .await;

    let tmp = write_blocklist(&[]);
    let blocklist = Blocklist::new(AddressSets::default());
    let registry = SourceRegistry::new(
        blocklist.clone(),
        tmp.path().to_path_buf(),
        vec![FeedSpec { name: "bad".into(), url: feed_server.uri() }],
    )
    .unwrap();
    assert!(registry.refresh_feeds(&Client::new()).await[0].is_err());
    assert_eq!(blocklist.len(), 0);
}

#[tokio::test]
async fn test_wasm_rule_sees_compute_budget_through_proxy() {
    let plugin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/zero-fee-filter.wasm");
    if !plugin.exists() {
        eprintln!("SKIP: plugins/zero-fee-filter.wasm not present");
        return;
    }
    let engine = WasmEngine::load(&plugin).unwrap();
    let mock = ok_downstream(json!("paidSig")).await;
    let proxy_url = serve(Blocklist::new(AddressSets::default()), mock.uri(), Some(engine)).await;

    let cb = solana_sdk::pubkey!("ComputeBudget111111111111111111111111111111");
    let tx = |price: u64| {
        let mut limit = vec![2u8];
        limit.extend_from_slice(&1_000_000u32.to_le_bytes());
        let mut ixs = vec![CompiledInstruction { program_id_index: 1, accounts: vec![], data: limit }];
        if price > 0 {
            let mut p = vec![3u8];
            p.extend_from_slice(&price.to_le_bytes());
            ixs.push(CompiledInstruction { program_id_index: 1, accounts: vec![], data: p });
        }
        STANDARD.encode(solana_tx_bytes(vec![Pubkey::new_unique(), cb], ixs, 1))
    };

    let spam: Value = post(&proxy_url, rpc_body("sendTransaction", json!([tx(0), {"encoding":"base64"}]), 1))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(spam["error"]["code"], -32002);
    assert_eq!(spam["error"]["data"]["rule_id"], "zero-fee-spam");

    let paid: Value = post(&proxy_url, rpc_body("sendTransaction", json!([tx(5_000), {"encoding":"base64"}]), 2))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(paid["result"], "paidSig");
}

# Aegis-SVM Implementation Plan

## Stack

- **Runtime:** Tokio
- **HTTP server:** axum
- **Transaction deserialization:** solana-sdk (VersionedTransaction)
- **Blocklist lookup:** ahash::AHashSet<[u8; 32]>
- **Metrics:** prometheus crate + /metrics endpoint
- **Wasm engine (stretch):** extism 1.13+
- **Benchmarks:** criterion
- **Demo infra:** Docker Compose (aegis + solana-test-validator + prometheus + grafana)

---

## Phase 1 — Cargo Workspace Scaffold

**Goal:** Compiling workspace with three crates and all dependencies wired.

### Tasks

1. `cargo new --workspace aegis-svm` at repo root
2. Create `crates/aegis-core/` — library crate, no I/O
3. Create `crates/aegis-proxy/` — binary crate, axum server
4. Create `crates/aegis-bench/` — binary crate, criterion benches
5. Add to root `Cargo.toml`:
   ```toml
   [workspace]
   members = ["crates/aegis-core", "crates/aegis-proxy", "crates/aegis-bench"]
   resolver = "2"
   ```
6. Add deps to `aegis-core/Cargo.toml`:
   ```toml
   solana-sdk = "2"
   ahash = "0.8"
   base64 = "0.22"
   bincode = "1"
   serde = { version = "1", features = ["derive"] }
   serde_json = "1"
   thiserror = "2"
   ```
7. Add deps to `aegis-proxy/Cargo.toml`:
   ```toml
   aegis-core = { path = "../aegis-core" }
   tokio = { version = "1", features = ["full"] }
   axum = "0.8"
   reqwest = { version = "0.12", features = ["json"] }
   prometheus = "0.13"
   tracing = "0.1"
   tracing-subscriber = { version = "0.3", features = ["json"] }
   config = "0.14"
   signal-hook = "0.3"
   ```
8. Add deps to `aegis-bench/Cargo.toml`:
   ```toml
   aegis-core = { path = "../aegis-core" }
   criterion = { version = "0.5", features = ["html_reports"] }
   ```
9. `cargo build --workspace` — must compile clean

**Done when:** `cargo build --workspace` succeeds with zero errors.

---

## Phase 2 — aegis-core: Types and Parser Stage

**Goal:** Given a raw base64 `sendTransaction` params string, produce an `InspectedTx` containing all pubkeys needed for screening.

### Types to define in `aegis-core/src/lib.rs`

```rust
pub struct InspectedTx {
    pub static_keys: Vec<[u8; 32]>,    // all static account keys
    pub signers: Vec<[u8; 32]>,         // first num_required_signers
    pub writable: Vec<[u8; 32]>,        // writable non-signers
    pub program_ids: Vec<[u8; 32]>,     // programs invoked
    pub lut_refs: Vec<[u8; 32]>,        // LUT account keys (v0 only), for cache lookup
}

pub enum Verdict {
    Allow,
    Block { rule_id: String, code: i32, message: String },
    Drop,
}

pub struct RejectionResponse {
    pub jsonrpc: &'static str,          // "2.0"
    pub error: RejectionError,
    pub id: serde_json::Value,
}
pub struct RejectionError {
    pub code: i32,
    pub message: String,
    pub data: RejectionData,
}
pub struct RejectionData {
    pub rule_id: String,
    pub trace_id: String,
}
```

### Tasks

1. Implement `parse_transaction(b64: &str) -> Result<InspectedTx, ParseError>`:
   - base64 decode
   - bincode deserialize into `VersionedTransaction`
   - match on `VersionedMessage::Legacy` vs `VersionedMessage::V0`
   - extract static_keys, signers (via `header.num_required_signers`), writable, program_ids
   - for v0: populate `lut_refs` from `address_table_lookups[*].account_key`
2. Unit tests:
   - `test_parse_legacy_tx` — hardcoded legacy tx bytes, assert signers/programs extracted
   - `test_parse_v0_tx` — hardcoded v0 tx bytes, assert lut_refs populated
   - `test_parse_invalid_b64` — assert `Err(ParseError::InvalidBase64)`
   - `test_parse_garbage_bytes` — assert `Err(ParseError::Deserialize)`

**Done when:** all unit tests pass, `cargo test -p aegis-core` green.

---

## Phase 3 — aegis-core: Static Engine

**Goal:** Load a blocklist file into an `AHashSet` and screen an `InspectedTx` against it.

### Tasks

1. Implement `Blocklist` struct:
   ```rust
   pub struct Blocklist {
       inner: Arc<ArcSwap<AHashSet<[u8; 32]>>>,
   }
   impl Blocklist {
       pub fn load_from_file(path: &Path) -> Result<Self, BlocklistError>
       pub fn reload(&self, path: &Path) -> Result<(), BlocklistError>
       pub fn check(&self, tx: &InspectedTx) -> Option<Verdict>
   }
   ```
   - `load_from_file`: read file, parse each line as base58 pubkey → `[u8; 32]`, build AHashSet
   - `reload`: build new set, atomically swap via `arc-swap` crate
   - `check`: iterate `tx.static_keys`; if any hit → `Verdict::Block { rule_id: "ofac-sdn", code: -32001, ... }`
2. Add `arc-swap = "1"` to aegis-core deps
3. Add `bs58 = "0.5"` to aegis-core deps (base58 pubkey parsing)
4. Ship a `blocklist.txt` fixture in `crates/aegis-core/tests/fixtures/` with 5 fake pubkeys
5. Unit tests:
   - `test_blocklist_loads` — load fixture, assert size == 5
   - `test_blocklist_hit` — tx with a key from fixture → `Verdict::Block`
   - `test_blocklist_miss` — tx with random key → `None`
   - `test_blocklist_reload` — write new file, call reload, assert new key is screened

**Done when:** all unit tests pass.

---

## Phase 4 — aegis-proxy: axum Server + Pipeline

**Goal:** Working proxy that listens on port 8899, intercepts `sendTransaction`, runs the pipeline, returns JSON-RPC errors for blocked txs, forwards everything else.

### Config (via `config` crate, env vars override)

```toml
# aegis.toml
listen_addr = "0.0.0.0:8899"
metrics_addr = "0.0.0.0:9090"
downstream_url = "http://127.0.0.1:8900"
blocklist_path = "blocklist.txt"
```

### Tasks

1. Implement `AppState`:
   ```rust
   struct AppState {
       blocklist: Blocklist,
       downstream: reqwest::Client,
       downstream_url: String,
       metrics: Metrics,          // Prometheus registry + handles
   }
   ```
2. Implement `Metrics` struct with all 5 metrics from TICKET-013 registered in a `prometheus::Registry`
3. Implement main axum handler:
   - `POST /` — buffer body, parse JSON-RPC `method`
   - If `sendTransaction` or `sendVersionedTransaction`:
     - extract base64 params[0]
     - call `parse_transaction` → record `aegis_inspection_duration_microseconds`
     - call `blocklist.check` → on `Verdict::Block`, return `RejectionResponse` JSON, increment counters
     - on `Verdict::Allow`, forward to downstream, return response
   - All other methods: transparent proxy (forward raw bytes, return raw response)
4. Implement `GET /metrics` handler returning Prometheus text format
5. SIGHUP handler: spawn tokio task listening on `signal_hook::consts::SIGHUP`, calls `blocklist.reload(path)`
6. Integration test `tests/proxy_test.rs`:
   - Spin up axum on random port in test
   - `test_blocked_tx_returns_error` — POST a tx with a blocklisted key, assert HTTP 200 with JSON-RPC error `code: -32001`
   - `test_clean_tx_forwards` — POST a clean tx, assert forwarded (mock downstream with `wiremock`)
   - `test_passthrough_method` — POST `getBalance`, assert forwarded unchanged

**Done when:** integration tests pass, `cargo run -p aegis-proxy` starts and accepts connections.

---

## Phase 5 — Criterion Benchmarks

**Goal:** Reproducible benchmark proving sub-50µs for stages 1+2.

### Tasks

1. In `aegis-bench/benches/pipeline.rs`:
   ```rust
   fn bench_parse_transaction(c: &mut Criterion) { ... }      // Stage 1 alone
   fn bench_static_engine(c: &mut Criterion) { ... }           // Stage 2 alone
   fn bench_full_pipeline(c: &mut Criterion) { ... }           // Stages 1+2 combined
   ```
2. Use a pre-serialized transaction fixture (1000 accounts, realistic size) — store in `benches/fixtures/`
3. Pre-warm blocklist with 50,000 entries before bench runs
4. `cargo bench -p aegis-bench` must show p50 < 50µs for `bench_full_pipeline`

**Done when:** `cargo bench` runs clean and numbers are documented in README.

---

## Phase 6 — Docker Compose + Demo Script

**Goal:** `docker compose up` starts full stack; demo script proves the three-way attack test.

### Tasks

1. Write `Dockerfile` for aegis-proxy:
   - `FROM rust:1.82 AS builder` → `cargo build --release -p aegis-proxy`
   - `FROM debian:bookworm-slim` → copy binary + blocklist.txt
2. Write `docker-compose.yml` per TICKET-014 topology
3. Write `infra/prometheus.yml`:
   ```yaml
   scrape_configs:
     - job_name: aegis
       scrape_interval: 5s
       static_configs:
         - targets: ['aegis:9090']
   ```
4. Write Grafana provisioning:
   - `infra/grafana/provisioning/datasources/prometheus.yml`
   - `infra/grafana/provisioning/dashboards/aegis.json` — pre-built dashboard with 4 panels: TPS, block rate, p99 latency, blocklist size
5. Write `demo/demo.sh`:
   ```bash
   #!/bin/bash
   # Tx 1: clean swap — should pass
   # Tx 2: OFAC-sanctioned signer — should return -32001
   # Tx 3: (placeholder for future Wasm CU rule)
   ```
   Uses `solana` CLI or `curl` to construct and send transactions

**Done when:** `docker compose up` starts all four services, Grafana dashboard loads at localhost:3000, demo script shows block/allow correctly.

---

## Phase 7 — Wasm Engine (Stretch)

**Goal:** A YAML-configured Extism plugin is evaluated as Stage 3 of the Inspection Pipeline.

> **Latency note:** Extism adds ~1ms per call. Stage 3 is not part of the sub-50µs claim. Benchmarks and README must be explicit: "Static pipeline: sub-50µs. With Wasm rules: ~1ms (still 100–200x faster than hosted threat intel APIs)."

### Tasks

1. Add `extism = "1"` to `aegis-core` deps
2. Define Wasm plugin input/output JSON schema:
   ```json
   // Input to plugin
   { "signers": ["pubkey..."], "programs": ["pubkey..."], "cu_requested": 200000, "priority_fee": 0 }
   // Output from plugin
   { "verdict": "allow" | "block" | "drop", "rule_id": "...", "message": "..." }
   ```
3. Implement `WasmEngine` in `aegis-core`:
   - Load plugin from path at startup
   - Call `plugin.call("evaluate", &input_json)` → parse output → `Verdict`
   - Reload on SIGHUP alongside blocklist
4. Wire `WasmEngine` as Stage 3 in the proxy pipeline (after Static Engine)
5. Write a sample Rust guest plugin in `plugins/zero-fee-filter/` using `extism-pdk`:
   - Blocks txs where `cu_requested > 400_000 && priority_fee == 0`
6. Add integration test: load zero-fee-filter plugin, send matching tx → assert `Verdict::Block`

**Done when:** `WASM_PLUGIN_PATH=plugins/zero-fee-filter.wasm cargo run -p aegis-proxy` blocks zero-fee spam transactions.

---

## File Tree (Target State)

```
aegis-svm/
├── Cargo.toml                          # workspace root
├── blocklist.txt                       # OFAC + exploiter pubkeys (one per line)
├── aegis.toml                          # default config
├── crates/
│   ├── aegis-core/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                  # re-exports
│   │   │   ├── parser.rs               # parse_transaction()
│   │   │   ├── blocklist.rs            # Blocklist struct
│   │   │   ├── types.rs                # InspectedTx, Verdict, RejectionResponse
│   │   │   └── wasm.rs                 # WasmEngine (Phase 7)
│   │   └── tests/
│   │       └── fixtures/
│   ├── aegis-proxy/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── config.rs
│   │   │   ├── handler.rs
│   │   │   └── metrics.rs
│   │   └── tests/
│   │       └── proxy_test.rs
│   └── aegis-bench/
│       ├── Cargo.toml
│       └── benches/
│           ├── pipeline.rs
│           └── fixtures/
├── demo/
│   └── demo.sh
├── infra/
│   ├── prometheus.yml
│   └── grafana/provisioning/
│       ├── datasources/prometheus.yml
│       └── dashboards/aegis.json
├── plugins/
│   └── zero-fee-filter/                # Phase 7
├── Dockerfile
└── docker-compose.yml
```

---

## Execution Order

```
Phase 1 (scaffold) → Phase 2 (parser) → Phase 3 (blocklist) → Phase 4 (proxy)
                                                                      ↓
                                                 Phase 5 (bench) ← Phase 4
                                                 Phase 6 (docker) ← Phase 4
                                                 Phase 7 (wasm) ← Phase 3 + Phase 4
```

Phases 5, 6, and 7 can start in parallel once Phase 4 is complete.

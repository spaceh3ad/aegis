# Aegis-SVM

**Open-core, sub-microsecond Solana JSON-RPC reverse proxy that intercepts and screens `sendTransaction` traffic before it reaches the validator.**

---

## The Problem

| Solution | Latency | Flaw |
|---|---|---|
| Blowfish / TRM REST APIs | 80–250 ms | Destroys Solana slot execution |
| Wallet extensions | 0 ms | Client-side only — bypassed trivially |
| Helius / QuickNode WAFs | < 1 ms | Inspect HTTP metadata, not tx internals |
| On-chain blacklists | 0 ms | Rent cost prohibitive; CU overhead |

Aegis-SVM operates at the **RPC ingress layer** — inline, sub-millisecond, and decoding the full transaction before it touches the validator.

---

## Benchmarks

Measured on Apple M-series (aarch64). Run yourself: `cargo bench -p aegis-bench`

| Stage | p50 Latency |
|---|---|
| Stage 1 — Parse (4-key v0 tx) | **627 ns** |
| Stage 2 — Blocklist check (allow) | **15 ns** |
| Stage 2 — Blocklist check (block) | **69 ns** |
| **Full pipeline (parse + check)** | **652 ns** |
| Stage 3 — Wasm rule engine | ~1 ms |

Static pipeline overhead: **76× under the 50 µs target.**  
With Wasm rules: ~1 ms — still 100–200× faster than hosted threat intel REST APIs.

---

## Architecture

```
[ RPC Client ]
      │
      ▼
┌─────────────────────────────────────────┐
│            AEGIS-SVM PROXY              │
│                                         │
│  1. JSON-RPC parse → method dispatch    │
│  2. base64 decode → VersionedTx         │
│     extract: signers, programs, LUTs    │
│  3. AHashSet<[u8;32]> blocklist check   │  ← sub-µs
│  4. Optional: Extism Wasm rule engine   │  ← ~1 ms
│                                         │
│  Block → JSON-RPC error (-32001/-32002) │
│  Allow → forward to downstream RPC      │
└─────────────────────────────────────────┘
      │
      ▼
[ solana-validator / TPU ]
```

**Hot reload:** Send `SIGHUP` to atomically swap the blocklist and Wasm plugin without dropping connections.

**Metrics:** Prometheus endpoint at `/metrics` — inspection latency histogram (µs buckets), tx/s by verdict, downstream health.

---

## Quick Start

### Local (no Docker)

```bash
# Build
cargo build --release -p aegis-proxy

# Run (forwards to local validator on :8900)
BLOCKLIST_PATH=blocklist.txt ./target/release/aegis-proxy

# With Wasm rules
WASM_PLUGIN_PATH=plugins/zero-fee-filter.wasm ./target/release/aegis-proxy

# Test
curl -X POST http://localhost:8899 \
  -H "content-type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getBalance","params":["11111111111111111111111111111111"]}'
```

### Docker Compose (full stack)

```bash
docker compose up
```

Starts four services:
- `aegis` — proxy on `:8899`
- `validator` — `solana-test-validator` on `:8900`
- `prometheus` — metrics scraper on `:9090`
- `grafana` — dashboard at `http://localhost:3000`

### Demo script

```bash
# Against local proxy:
bash demo/demo.sh http://localhost:8899

# Against Docker:
docker compose up -d
bash demo/demo.sh
```

---

## Configuration

| Env var | Default | Description |
|---|---|---|
| `LISTEN_ADDR` | `0.0.0.0:8899` | Proxy bind address |
| `DOWNSTREAM_URL` | `http://127.0.0.1:8900` | Validator RPC URL |
| `BLOCKLIST_PATH` | `blocklist.txt` | One base58 pubkey per line |
| `WASM_PLUGIN_PATH` | _(unset)_ | Path to `.wasm` rule plugin |

---

## Blocklist Format

```
# Lines starting with # are ignored
So11111111111111111111111111111111111111112
EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
```

Reload without restart: `kill -HUP <pid>`

---

## Writing Custom Wasm Rules

Rules are compiled to `wasm32-wasip1` using [extism-pdk](https://github.com/extism/extism):

```rust
// plugins/my-rule/src/lib.rs
use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    signers: Vec<String>,
    programs: Vec<String>,
    cu_requested: u64,
    priority_fee: u64,
}

#[derive(Serialize)]
struct Output { verdict: &'static str, rule_id: &'static str, message: &'static str }

#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let input = input.0;
    if input.cu_requested > 200_000 && input.priority_fee == 0 {
        Ok(Json(Output { verdict: "block", rule_id: "my-rule", message: "blocked" }))
    } else {
        Ok(Json(Output { verdict: "allow", rule_id: "", message: "" }))
    }
}
```

```bash
cargo build --release --target wasm32-wasip1
WASM_PLUGIN_PATH=target/wasm32-wasip1/release/my_rule.wasm ./target/release/aegis-proxy
```

The included `plugins/zero-fee-filter.wasm` blocks `cu_requested > 400_000 && priority_fee == 0`.

---

## Crate Structure

```
crates/
  aegis-core/     — pure library: parser, blocklist, Wasm engine
  aegis-proxy/    — axum server, config, metrics, request pipeline
  aegis-bench/    — Criterion benchmarks
plugins/
  zero-fee-filter/  — sample Wasm rule (Rust guest)
  zero-fee-filter.wasm  — pre-built plugin binary
infra/
  prometheus.yml
  grafana/provisioning/
demo/
  demo.sh
```

---

## Running Tests

```bash
cargo test --workspace
```

13 tests: 9 unit (parser, blocklist, Wasm engine) + 4 integration (proxy pipeline, metrics, OFAC block).

---

## Roadmap

- [ ] Address Lookup Table cache (Geyser/Yellowstone sync for v0 LUT expansion)
- [ ] Staked connection pass-through (SWQoS / Jito forwarding)
- [ ] Auto-updating OFAC/threat intel feeds (Chainalysis, TRM, Elliptic)
- [ ] HA clustering with distributed rate limiting
- [ ] Compliance audit dashboard with signed rejection receipts

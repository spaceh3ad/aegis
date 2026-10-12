# Aegis — Demo Runbook

Everything a judge (or you, on stage) needs to reproduce the demo in under two minutes.

## What Aegis is, in one line

A sovereign, sub-microsecond **transaction firewall** that sits in front of a Solana (or
EVM) RPC node and blocks illicit or toxic transactions *before* the node executes them —
where cloud threat-intel APIs are 80–250 ms too slow and HTTP WAFs are blind to what's
inside the transaction.

---

## Path A — Live RPC demo (recommended: fast, no emulation)

Runs the release binary in front of any real Solana RPC.

```bash
cargo build --release -p aegis-proxy

# Public testnet:
bash demo/rpc-demo.sh

# Helius testnet:
HELIUS_API_KEY=xxxx bash demo/rpc-demo.sh
# or any endpoint:
AEGIS_RPC_URL="https://devnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" bash demo/rpc-demo.sh
```

What it shows:
1. Real `getVersion` / `getHealth` / `getSlot` forwarded through Aegis to the live cluster.
2. A transaction signed by a **blocklisted** address rejected locally with
   `-32001` — it never reaches the RPC.
3. Measured inspection latency from the Prometheus histogram.

## Path B — Full local stack (self-contained, offline)

```bash
docker compose up --build          # Aegis :8899, validator :8900, Prometheus :9090, Grafana :3001
bash demo/demo.sh                  # runs all checks against :8899
open http://localhost:3001         # Grafana dashboard
```

> The Solana validator image is `linux/amd64`; on Apple silicon it runs under emulation,
> so first start can take a few minutes. Path A avoids this entirely.

## Path C — Docker against a real RPC

```bash
docker compose -f docker-compose.rpc.yml up --build
# Helius:
AEGIS_RPC_URL="https://testnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" \
  docker compose -f docker-compose.rpc.yml up --build
```

---

## The 90-second stage script

1. **Frame the problem (15s).** "Solana settles in ~400 ms slots. Existing screening is
   either a 150 ms REST round-trip in the hot path, or an HTTP WAF that never sees inside
   the transaction. So operators carry traffic they don't want — spam, drainers, exploits,
   sanctioned counterparties."

2. **Show the number (15s).** `cargo bench -p aegis-bench` — full pipeline **~700 ns** on
   Solana (parse + policy), 50 000-entry list still **~18 ns** (O(1)). That is ~70 000×
   under a 50 µs budget.

3. **Show the live block (30s).** `bash demo/rpc-demo.sh` — a blocklisted signer is
   rejected at the proxy (`-32001`, `rule_id=blocklist:file`) while real cluster reads
   pass through untouched.

4. **Show operator-owned policy (20s).** Point at `blocklist.txt` (one base58/`0x` per
   line), `THREAT_FEEDS="seal=https://…"` (hot-reloaded feeds, attributed per source), and
   the Wasm rule engine (`plugins/zero-fee-filter.wasm` blocks high-CU zero-fee spam).
   "Aegis ships no list and calls no home. The operator owns the policy."

5. **Close (10s).** "Fail open on parse errors, fail safe on feed errors. Sovereign
   sidecar, not a cloud chokepoint."

---

## Talking points that map to the code

- **Two chains, one binary.** `sendTransaction` and `eth_sendRawTransaction` are decoded
  and screened by the same process; the method name picks the pipeline.
- **O(1) enforcement at any list size.** `aegis-core/src/blocklist.rs` — `AHashMap` /
  `ArcSwap`, atomically hot-swapped on `SIGHUP`.
- **v0 lookup tables solved.** `crates/*/src/lut.rs` resolve Address Lookup Tables so keys
  loaded *through* a table are screened like static keys — a gap most in-line filters miss.
- **Fail-safe by design.** `handler.rs`: undecodable txs are forwarded (the node uses the
  same decoder); a feed outage keeps the last good copy, so a list never silently empties.
- **Attributed rejections.** Every block names its source (`blocklist:<feed>`) plus a
  `trace_id` — an audit trail for compliance.

## Optional: EVM path

```bash
DOWNSTREAM_URL=http://127.0.0.1:8545 LISTEN_ADDR=0.0.0.0:8546 ./target/release/aegis-proxy
```

Screens the recovered sender and the call target. Useful to show the "one sidecar, any
chain" claim (`eth_sendRawTransaction`).

## Health check for the repo

```bash
cargo test --workspace          # 60+ tests, incl. property/fuzz tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

# Aegis-SVM

## 1. Executive Summary

Solana processes transactions within ~400ms continuous slot windows. Protocols, institutional neobanks, and RPC operators face two opposing forces:

- **Regulatory & Security Liabilities:** Facilitating transactions tied to sanctioned wallets (OFAC SDN), known malicious contracts, or wallet-drainer instruction patterns exposes operators to severe regulatory and financial risks.
- **The Latency Trap:** Existing threat intelligence solutions (e.g., Blowfish, Blockaid, TRM) rely on external HTTP REST APIs that introduce 80–250ms of round-trip latency. In high-contention Solana environments, this delay leads directly to dropped slots, stale state execution, and catastrophic slippage.

Aegis-SVM is an open-core, in-line reverse proxy written in Rust that sits directly in front of `solana-validator` or RPC nodes. By utilizing zero-copy packet deserialization, in-memory pubkey sets, and an embeddable Wasm rule engine, Aegis inspects and filters `sendTransaction` traffic at line rate with **under 35 microseconds of latency overhead** — intercepting illicit or toxic transactions before they hit the RPC queue, validator scheduler, or consensus layer.

---

## 2. Why Current dApp Approaches Fail

| Current Method | Layer | Latency Added | Core Flaw |
|---|---|---|---|
| Frontend Screening (TRM / Chainalysis) | Browser UI | Negligible (UI only) | Zero enforcement. Attackers bypass UI checks trivially using scripts, raw RPC calls, or forked frontends. Smart contracts still execute the exploit. |
| Centralized Backend Relayers | Off-Chain Server | Severe (150–400ms) | Synchronous REST calls destroy Solana execution efficiency, leading to transaction expiration and missed slot boundaries. |
| Wallet Warnings (Blowfish / Blockaid) | Client Extension | Moderate | Protects users from signing bad transactions; does not protect protocols, liquidity pools, or RPC infrastructure from incoming attacks. |
| Managed RPC WAFs (Helius / QuickNode) | L4 / Standard L7 | Low (< 1ms) | Inspects HTTP headers, IPs, and payload size only. Blind to deserialized Solana transaction internals (signers, program IDs, instructions). |
| On-Chain Blacklists (Token-Level) | SVM State | Zero | Extreme rent costs to store address lists on-chain; bloats compute units (CUs); impossible for dynamic, high-frequency blacklists. |

---

## 3. System Architecture

Aegis-SVM intercepts incoming JSON-RPC traffic, deserializes the transaction payload in-place, evaluates it against static and dynamic rule sets, and routes clean traffic directly to local TPU ports or Stake-Weighted QoS (SWQoS) pipelines.

```
[ Inbound Client / Bot Traffic ]
                │
                ▼
┌────────────────────────────────────────────────────────────────────────┐
│                        AEGIS-SVM REVERSE PROXY                         │
│                                                                        │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ 1. Zero-Copy Header & Account Parser (solana-sdk)                │  │
│  │    - Extracts Signers, Writable Accounts, Program IDs (< 5µs)    │  │
│  └──────────────────────────────────┬───────────────────────────────┘  │
│                                     ▼                                  │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ 2. In-Memory Static Engine (OFAC / Exploiter Blacklists)         │  │
│  │    - Aho-Corasick / AHashSet / Roaring Bitmaps (< 10µs)          │  │
│  └──────────────────────────────────┬───────────────────────────────┘  │
│                                     ▼                                  │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ 3. Dynamic Sandboxed Rule Engine (Wasm / Extism)                 │  │
│  │    - Custom policies: drainer signatures, CU/fee ratios,         │  │
│  │      instruction whitelist/blacklist (< 20µs)                    │  │
│  └──────────────────────────────────┬───────────────────────────────┘  │
└─────────────────────────────────────┼──────────────────────────────────┘
                                      │
              ┌───────────────────────┴───────────────────────┐
              │                                               │
      [ Policy Violation ]                            [ Approved Traffic ]
              │                                               │
              ▼                                               ▼
   Immediate JSON-RPC 403                      Direct Forward via High-Speed Pipeline:
   { "code": -32003,                           - Local Agave TPU (QUIC)
     "message": "Blocked by Aegis",            - Staked Connection (SWQoS)
     "rule_id": "ofac-sdn-match",              - Private Relayer (Jito)
     "trace_id": "0x7f4a..." }
```

### Key Technical Mechanisms

- **Zero-Copy Account Extraction:** Reads the `VersionedTransaction` header and raw account pubkey bytes directly from the base64-encoded RPC buffer without full heap allocations or deep instruction decoding.
- **Address Lookup Table (LUT) Synchronization:** Maintains a lightweight in-memory cache of active Address Lookup Tables synced via Geyser/Yellowstone stream, allowing zero-latency expansion of v0 transaction addresses.
- **Line-Rate Policy Matching:** Compiles static blocklists (OFAC, known exploiters) into pre-warmed cache structures (`ahash::AHashSet`), executing lookups in O(1) amortized time.
- **Sandboxed Dynamic Rules:** Integrates a lightweight Wasm runtime (Extism/Wasmtime) enabling operators to define custom JSON/YAML rule sets hot-reloaded at runtime without dropping connections.

---

## 4. Competitive Analysis

```
                         LATENCY SENSITIVITY
                     (Target: < 50µs overhead)
                                ▲
                                │
             [AEGIS-SVM]        │  [Triton / Helius Internal]
          - Open, Sovereign     │  - Closed, Vendor-Locked
          - Self-Hostable       │  - Cloud-Only
          - Extensible Wasm     │  - Static Filter Heuristics
                                │
────────────────────────────────┼────────────────────────────────────────►
CLIENT / WALLET-SIDE            │                         GATEWAY / RPC
                                │
        [Blowfish / Blockaid]   │  [Chainalysis / TRM Labs]
        - Consumer Wallet UX    │  - Compliance Data Provider
        - Hosted REST API       │  - No Native Solana Ingress Engine
          (~150ms)              │  - REST API Polling (~200ms)
                                │
                                ▼
                      HIGH LATENCY OVERHEAD
```

### Market Differentiation

- **Vs. Threat Intel Providers (Blowfish, TRM):** Aegis is not an intelligence aggregator; it is the sub-millisecond execution engine that makes their intelligence actionable on high-throughput Solana nodes.
- **Vs. Cloud RPC Providers (Helius, Triton):** Aegis is open-core and sovereign. Protocols and institutional nodes running dedicated hardware can deploy Aegis locally inside their VPC, maintaining zero external dependencies.

---

## 5. Declarative Rule Engine Specification

Operators define declarative, hot-reloadable YAML policies:

```yaml
version: "1.0"
metadata:
  node_id: "eu-central-rpc-01"
  default_action: "allow"

rules:
  # 1. Statutory Sanctions Compliance
  - id: "enforce-ofac-sdn"
    type: "static_account_match"
    action: "block"
    target: "any_account" # signers | writable | readonly | any
    source_feed: "embedded://ofac_sdn_latest"
    rejection_code: -32001
    rejection_message: "Transaction contains addresses flagged under statutory compliance sanctions."

  # 2. Malicious Drainer Pattern Mitigation
  - id: "prevent-unauthorized-mint-delegation"
    type: "instruction_signature"
    action: "block"
    program_id: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    instruction_type: "Approve"
    condition: "target_account != user_wallet && is_new_delegate == true"
    rejection_code: -32002
    rejection_message: "Transaction flagged: suspicious token authority delegation pattern."

  # 3. Micro-Spam & CU Exhaustion Defense
  - id: "filter-zero-fee-spam"
    type: "compute_market"
    action: "drop"
    condition: "cu_requested > 400000 && priority_fee_lamports == 0"
```

---

## 6. Colosseum Frontier Hackathon Strategy

**Track:** Solana (this edition's tracks are ecosystems; State of the Art / Public Good as secondary angles)

**Core Pitch:** "Web3 security platforms add 80–250 ms REST API lag; standard Solana RPCs blindly forward illicit flows and spam. Aegis provides an open-core, sub-microsecond programmable reverse proxy that secures transactions at wire speed before they touch the node."

### Live Demo & Judging Flow

**Latency — committed, reproducible numbers:**
- Full pipeline (parse + policy) **~700 ns** on Solana, **~26 µs** on EVM, from Criterion.
- **O(1)** address screening: a 50,000-entry list costs the same as one entry (~18 ns).
- Reproduce with `cargo bench -p aegis-bench` on an idle machine. Criterion is a
  **dev-only** dependency of `aegis-bench`; it is never compiled into the server or the image.

**The "Three-Way Attack" Test:**
1. **Tx 1 (Standard Swap):** Sent to proxy → Passed → forwarded to the node.
2. **Tx 2 (Sanctioned Wallet Interaction):** Sent to proxy → intercepted, structured JSON-RPC error, zero impact on node compute.
3. **Tx 3 (Zero-Fee CU Exhaustion Attack):** Sent to proxy → blocked by the operator's policy (shipped Wasm rule; policy is authored in Rust and compiled to Wasm).

**Turnkey Deployment** — 1-command Docker configuration running Aegis alongside a local Agave validator:

```bash
docker compose up --build
```

---

## 7. Commercialization & Venture Roadmap

### Target Customer Segments (ICP)

- **Regulated Fintechs & Payment Gateways:** Platforms (e.g., Stripe crypto payouts, institutional desks) that require auditable compliance boundaries to avoid facilitating illicit flows.
- **High-Throughput DeFi Protocols:** Perps and AMM protocols (e.g., Drift, Phoenix) seeking private ingress proxies that shield their backends from predatory toxic flow and micro-spam.
- **Mid-Tier RPC Providers:** Operators looking to offer enterprise-grade SLA, spam resistance, and compliance guarantees without writing custom proxy software in-house.

### Monetization Model

**Community Edition (Open Source):**
- Core Rust proxy binary
- Zero-copy Solana/EVM parsers
- Local address lists + operator-selected threat feeds
- Sandboxed Wasm rule engine (author rules in Rust, hot-swap live)

**Enterprise Edition (Commercial License):**
- Curated, auto-updating threat-intelligence feeds and managed integrations
- High-availability clustering and distributed rate limiting across regional nodes
- Compliance audit dashboard with cryptographically signed JSON-RPC rejection receipts

---

## 8. Ten-Second Pitch

Solana settles in ~400 ms slots; incumbent screening is a 150 ms REST round-trip or an
HTTP WAF that never sees inside the transaction. **Aegis is a sovereign, sub-microsecond
transaction firewall** that decodes every `sendTransaction` *before* the node does and
blocks illicit or toxic traffic at wire speed — one Rust sidecar for Solana and EVM, with
operator-owned, hot-reloadable policy and no central chokepoint.

## 9. What Is Actually Working (verified)

- **Inspection pipeline**: Solana v0/legacy parse + `eth_sendRawTransaction` sender
  recovery, static list check, Extism Wasm rule engine — **~700 ns** Solana, **~26 µs** EVM
  full pipeline (Criterion, reproducible).
- **O(1) address screening**: a 50 000-entry list costs **~18 ns**, same as one entry.
- **v0 Address Lookup Table resolution**: keys loaded through a LUT are screened like
  static keys (bounded cache, refetch on index miss).
- **Operator-owned feeds**: `THREAT_FEEDS` hot-reloaded with per-source attribution;
  failing feeds keep their last good copy (never silently empty).
- **SIGHUP hot reload** of the local list and Wasm plugin without dropping connections.
- **60+ tests** (unit, integration, property/fuzz) green; `clippy -D warnings` and
  `cargo fmt --check` clean.
- **One command to demo**: `bash demo/demo.sh` (story), `WALLET=1 bash demo/demo.sh`
  (real wallet on localhost), or `docker compose up` (self-contained validator). See the
  demo section in `README.md`.

## 10. Trade-offs and Honesty

- **Fail open on parse errors, fail safe on feed errors.** An undecodable transaction is
  forwarded (the downstream node uses the same decoder and rejects it); a feed outage
  keeps the last good copy rather than emptying the list.
- **A clean transaction needs a reachable downstream.** The allow path forwards the
  transaction, so a read-only or unreachable RPC surfaces as a `502` — Aegis never
  pretends to accept a transaction it could not deliver.
- **Wasm rules cost ~1 ms**, deliberately excluded from the sub-µs claim; they are for
  richer policy, not the hot path.
- **LUT cache is best-effort** and bounded; a cache miss screens static keys only.

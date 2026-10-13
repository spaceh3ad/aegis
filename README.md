# Aegis

**A programmable, wire-speed transaction firewall for RPC infrastructure. One sidecar binary for Solana and EVM chains.**

Aegis sits in front of your RPC node, the way Envoy or a WAF sits in front of a web app. It decodes every transaction submission (`sendTransaction`, `eth_sendRawTransaction`) and runs it through operator-defined policy before it reaches the node. That takes under 1 µs on Solana and about 26 µs on EVM.

---

## Why

RPC operators have to handle submitted transactions they don't want to carry: spam that burns compute, zero-fee bots competing for block space, drainer and exploit traffic, and, for regulated operators, sanctioned counterparties. The current options are either too slow or blind to what's inside the transaction:

| Solution | Latency | Flaw |
|---|---|---|
| Hosted threat-intel REST APIs | 80–250 ms | Too slow for the transaction hot path |
| Wallet extensions | 0 ms | Client-side only, trivially bypassed |
| HTTP-level WAFs | < 1 ms | See headers and IPs, not transaction internals |
| On-chain blacklists | 0 ms | Rent/gas cost, compute overhead, protocol-level |

### Threat coverage — what Aegis does and does not address

| Threat | Covered? | How / why not |
|---|---|---|
| **Network-level bot spam / congestion** | **Partially** | Rejects junk submissions at the RPC edge before the node ingests or forwards them — e.g. high-compute, zero-fee spam — in O(1) time that holds under burst. One operator's node stops carrying what it didn't accept; benefit compounds with adoption. Aegis sits in front of an **RPC node**, not the validator's TPU/QUIC ingestion, so it does not by itself police the global leader path. |
| **Exploit / drainer / sanctioned traffic** | **Yes** | Static address lists and operator-selected feeds are screened per transaction (signers, accounts, program/call targets), with source-attributed, auditable rejections. |
| **Wallet display spam (dusting, fake transfers, address poisoning)** | **No** | This is an **inbound, client-layer** problem: dust tokens airdropped *to* a user and lookalike entries in wallet history. Aegis screens outbound **transaction submission** (`sendTransaction`, `eth_sendRawTransaction`) and never sees a user's token list or history. Wallets and explorers own that layer. The only adjacency: if a poisoned destination is itself on an operator list, the outbound transfer is blocked before funds move. |

Naming the boundary is deliberate: it is the difference between a firewall and a claim.

## Design principles

- **Sovereign sidecar, not a cloud chokepoint.** Aegis runs on the operator's own hardware. No traffic is routed through us, and there is no central switch.
- **Operators own the policy.** Every rule, list, and feed is chosen and loaded by the operator. Aegis ships with nothing enabled.
- **Engine and data are separate.** Aegis does not curate address lists. It subscribes to whichever feeds the operator trusts (their own list, SEAL, TRM, Chainalysis, …), and every rejection names the source responsible (`blocklist:<feed>`).
- **Fail open on parse errors, fail safe on feed errors.** Undecodable transactions are forwarded, because the node rejects them with the same decoder. A feed outage keeps the last good copy, so a list never silently empties.

## Policy layers

```
Layer 3  Programmable Wasm rules      spam / fee policy / MEV / custom logic   ~1 ms
Layer 2  Threat-intel feeds           operator-selected, hot-reloaded          O(1)
Layer 1  Static address lists         local file, Solana + EVM in one format   O(1)
```

---

## Benchmarks

Measured on an idle **Apple M2 Pro (arm64), macOS, single core**, Rust stable, via
`cargo bench -p aegis-bench`.

> Absolute numbers are **hardware-dependent** — read them as orders of magnitude, not
> guarantees. Benchmark on an idle machine: Criterion compares against the previous run, so
> a busy box reports a "regression" even when nothing in the code changed.

| Stage | p50 |
|---|---|
| Solana: parse v0 tx (4 keys) | **722 ns** |
| Solana: address list check (allow) | **17 ns** |
| Solana: address list check, **50 000 entries** (allow) | **18 ns** |
| **Solana: full pipeline (parse + check)** | **700 ns** |
| EVM: RLP decode + secp256k1 signer recovery | **26 µs** |
| **EVM: full pipeline** | **26 µs** |
| Wasm rule engine (either chain) | ~1 ms |

The claims that matter here are **hardware-independent**, which is why they are the ones to hold onto:

- **O(1) lookup.** A 50 000-entry list costs the same as a 1-entry one (~1.1× on this machine).
  That is an algorithmic property — it holds on a Raspberry Pi.
- **Inside the hot path.** The full Solana pipeline is **sub-µs** on this class of CPU, versus
  **80–250 ms** for a hosted threat-intel REST call — 5–6 orders of magnitude, because one is a
  CPU cost and the other is a network round-trip. That gap holds everywhere.
- **Where the time goes.** Solana inspection is ~97% decode, ~3% policy. EVM cost is dominated by
  secp256k1 sender recovery, which is unavoidable when screening senders.
- **Throughput per core** (derived): roughly **1.4M Solana txs/s**, **~38k EVM txs/s**.

---

## Architecture

```
[ Client / wallet / bot ]
          │  JSON-RPC (single or batch)
          ▼
┌───────────────────────────────────────────────────────┐
│                         AEGIS                         │
│                                                       │
│  sendTransaction        base58/base64 → VersionedTx   │
│                         signers, programs, LUTs,      │
│                         ComputeBudget limit + price   │
│  eth_sendRawTransaction EIP-2718 → recover sender     │
│                         to, selector, gas, tip        │
│                                                       │
│  1. Address lists  (file ∪ feeds, per-source tagged)  │
│  2. Wasm rules     (chain-agnostic JSON input)        │
│                                                       │
│  Block → JSON-RPC error (-32001 list / -32002 rule)   │
│  Batch → blocked entries answered locally,            │
│          remainder forwarded as a smaller batch       │
│  Other methods → forwarded untouched                  │
└───────────────────────────────────────────────────────┘
          │
          ▼
[ Solana validator / RPC node   or   geth / reth / op-node ]
```

The same binary serves any chain. Point `DOWNSTREAM_URL` at a Solana RPC or an EVM execution client, and the method name picks the pipeline.

**Hot reload:** `SIGHUP` atomically swaps the local list and the Wasm plugin without dropping connections. Feeds refresh on their own interval.

**Metrics:** Prometheus at `/metrics`. Covers the inspection latency histogram, verdicts, blocks by `rule_id`, list size, downstream health, and feed refresh results.

---

## Quick start

```bash
cargo build --release -p aegis-proxy

# Solana (point DOWNSTREAM_URL at any JSON-RPC: local node, Helius, Triton, public cluster)
DOWNSTREAM_URL=https://api.testnet.solana.com ./target/release/aegis-proxy

# EVM (e.g. Base / Arbitrum / Ethereum node)
DOWNSTREAM_URL=http://127.0.0.1:8545 LISTEN_ADDR=0.0.0.0:8546 ./target/release/aegis-proxy

# With operator-selected threat feeds and a Wasm rule
THREAT_FEEDS="seal=https://example.org/seal.txt,internal=https://intel.corp/list.txt" \
WASM_PLUGIN_PATH=plugins/zero-fee-filter.wasm \
./target/release/aegis-proxy
```

`sendTransaction` is only enforced when `DOWNSTREAM_URL` is a node Aegis can actually
reach: the clean-path check forwards the transaction downstream, so an unreachable or
read-only RPC turns a clean transaction into a `502`. Aegis never *pretends* to accept a
transaction it could not have delivered.

### Docker Compose — against a real RPC

No local validator, no emulation. Runs Aegis in front of any Solana RPC (Helius,
Triton, public cluster):

```bash
# Public testnet:
docker compose -f docker-compose.rpc.yml up --build

# Helius testnet (key in the URL):
AEGIS_RPC_URL="https://testnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" \
  docker compose -f docker-compose.rpc.yml up --build

AEGIS_PROXY_URL=http://127.0.0.1:8899 bash demo/demo.sh
```

### Docker Compose — full local stack (self-contained, offline)

```bash
docker compose up --build
bash demo/demo.sh
```

Aegis `:8899`, `solana-test-validator` (host `:8900`), Prometheus `:9090`, Grafana
`http://localhost:3001` (dashboard uid `aegis-svm-v1`). The validator image is `linux/amd64`;
on Apple silicon it runs under emulation, so first start can take a few minutes. Only the
Docker stacks include Grafana.

### Demo script

One script, three modes. Point it at **any** RPC via `AEGIS_RPC_URL` / `HELIUS_API_KEY` — only
the URL changes; no Docker needed.

```bash
cargo build --release -p aegis-proxy

bash demo/demo.sh                                   # full story vs public devnet (pass/fail gate)
AEGIS_RPC_URL="https://devnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" bash demo/demo.sh
SPAM=500 bash demo/demo.sh                          # bigger spam burst (SPAM=0 to skip)
WALLET=1 bash demo/demo.sh                           # interactive: point a wallet here
PROXY=http://127.0.0.1:8899 bash demo/demo.sh        # target a running proxy (e.g. the Compose stack)
```

Automated beats: live reads pass through → blocklisted signer rejected → clean tx forwarded →
`SIGHUP` hot-reload flips a live tx allowed→blocked → Wasm spam rule drops a high-CU zero-fee
tx → spam **burst** (N blocked, 0 forwarded, compute never reserved) → cross-chain rule
hot-swap (`approve()` blocked, `transfer()` passes). Exit code is 0 only if every beat verified.

#### Wallet demo (`WALLET=1`)

A wallet broadcasts through the RPC configured for its network — not the dApp's provider. So
Aegis goes in the middle by **being the wallet's RPC endpoint**, exactly as a WAF replaces an
origin URL. Point the wallet at Aegis, send to a blocklisted address, and it is refused at the
edge; the transaction never reaches the chain.

```bash
WALLET=1 bash demo/demo.sh                       # Solana Devnet (Phantom)
CHAIN=base WALLET=1 bash demo/demo.sh            # Base Sepolia (Phantom/MetaMask EVM)
```

Phantom: **Settings → Developer Settings → enable testnet** → set the Solana Devnet RPC URL to
`http://127.0.0.1:8899`. MetaMask: **Settings → Networks → Base Sepolia → RPC URL** = the
printed URL. localhost is fine; `TUNNEL=1` adds a cloudflared `https` URL only if a wallet
rejects a non-https endpoint. One run fronts one upstream chain.

> MetaMask estimates gas via the same RPC before sending, so it shows a *gas-estimation* error
> rather than the raw JSON-RPC text. The rejection is still Aegis — watch the terminal log.

> Aegis rejections are identified by `error.data.rule_id` (present on its own rejections, absent
> on downstream node errors). JSON-RPC codes alone can collide with the chain — e.g. Solana
> itself returns `-32002` for a failed transaction simulation.

### Case-based demo (two terminals)

A scripted equivalent of the six cases, run side by side: **terminal 2** shows Aegis screening
live, **terminal 1** drives each case. No Docker; uses a local validator and `anvil`.

```bash
# terminal 2 — the stack + live log you watch
bash demo/proxy.sh

# terminal 1 — one-time, then run any case
bash demo/cases/setup.sh        # keys, LUT, Solana program, ERC-20 (needs solana + forge)
bash demo/cases/blacklist.sh    # list block + LUT-delivered sanction
bash demo/cases/spam.sh         # SPAM=100  zero-fee, high-CU burst
bash demo/cases/evm-approval.sh # approve()/increaseAllowance() refused
bash demo/cases/cross-chain.sh  # hot-swap the rule live: Solana spam -> EVM guard
bash demo/cases/silent-drop.sh  # HTTP 204: silent drop vs a visible block
bash demo/cases/program-selector.sh  # deny by program id / method selector
```

`proxy.sh` runs the release binary twice — `aegis(sol)` `:8899` → validator, `aegis(evm)`
`:8898` → anvil — with `LOG_FORMAT=pretty`, and tails the log so you can watch:

```
WARN BLOCKED rule=blocklist:file tx=svm target=- signer=<pk> what=cu=0 fee=0
INFO allowed tx=evm target=0xe7f1… signer=0xf39f… what=selector=0xa9059cbb
WARN DROPPED tx=svm target=Tokenkeg… what=cu=0 fee=0
INFO wasm plugin reloaded plugin=demo/.run/rule.wasm
```

`setup.sh` creates the fixtures the cases need (an Address Lookup Table holding the sanctioned
address, a Solana `payments-program`, an ERC-20 `DemoToken`) and records their addresses in
`demo/.run/` (gitignored).

---

## Configuration

| Env var | Default | Description |
|---|---|---|
| `LISTEN_ADDR` | `0.0.0.0:8899` | Proxy bind address |
| `DOWNSTREAM_URL` | `http://127.0.0.1:8900` | Solana RPC or EVM execution client |
| `BLOCKLIST_PATH` | `blocklist.txt` | Local address list (see format below) |
| `THREAT_FEEDS` | _(unset)_ | `name=url,name2=url2`. Each feed uses the same text format |
| `THREAT_FEED_INTERVAL_SECS` | `300` | Feed refresh interval |
| `WASM_PLUGIN_PATH` | _(unset)_ | Path to a `.wasm` rule plugin |

## Address list format

One address per line. Solana (base58) and EVM (`0x` hex) entries can be mixed, and `#` starts a comment. Invalid lines reject the whole file or feed, so a bad update is never partially applied.

```
# local operator list
CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv
0x098B716B8Aaf21512996dC57EB0615e2383E2f96
```

On EVM, both the recovered sender and the call target are screened.

---

## Writing Wasm rules

Rules are ordinary Rust (or any Extism-supported language) compiled to `wasm32-wasip1`. Each rule receives one chain-agnostic JSON object:

| Field | Solana | EVM |
|---|---|---|
| `chain` | `"svm"` | `"evm"` |
| `signers` | fee payer + co-signers (base58) | recovered sender (`0x…`) |
| `programs` | invoked program ids | call target (empty on create) |
| `cu_requested` | ComputeBudget unit limit (0 if unset) | gas limit |
| `priority_fee` | ComputeBudget unit price (0 if unset) | priority fee / legacy gas price |
| `chain_id`, `selector`, `value_wei` | _(absent)_ | chain id, 4-byte selector, value |

The rule returns `{"verdict": "allow" | "block" | "drop", "rule_id": "...", "message": "..."}`.

### Four worked examples ship in `plugins/`

| Plugin | Chain | Blocks | `rule_id` |
|---|---|---|---|
| `zero-fee-filter` | Solana | transactions requesting >400k CU with zero priority fee (spam) | `zero-fee-spam` |
| `approve-guard` | EVM | ERC-20 `approve` / `increaseAllowance` calls (drainer entry point) | `approve-guard` |
| `program-guard` | both | SVM: a denylisted program id; EVM: a denylisted method selector | `program-guard` |
| `silent-drop` | both | returns `drop` (HTTP 204) instead of a visible rejection | `silent-drop` |

Build any of them and point Aegis at it:

```bash
cd plugins/approve-guard
cargo build --release --target wasm32-wasip1
WASM_PLUGIN_PATH=target/wasm32-wasip1/release/approve_guard.wasm \
  ./target/release/aegis-proxy   # run from the repo root
```

The engine is generic — write a new rule by copying either plugin directory. The source
for `approve-guard` is 40 lines:

```rust
#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let blocked = input.0.chain == "evm"
        && input.0.selector.as_deref() == Some("0x095ea7b3"); // approve(address,uint256)
    Ok(Json(if blocked {
        Output { verdict: "block", rule_id: "approve-guard", message: "ERC-20 approval call blocked by policy" }
    } else {
        Output { verdict: "allow", rule_id: "", message: "" }
    }))
}
```

```bash
cargo build --release --target wasm32-wasip1
WASM_PLUGIN_PATH=target/wasm32-wasip1/release/my_rule.wasm ./target/release/aegis-proxy
```

The bundled `plugins/zero-fee-filter.wasm` blocks Solana transactions that request more than 400k CU with a zero priority fee.

---

## Crate structure

```
crates/
  aegis-core/   parsers (Solana, EVM), address lists, Wasm engine
  aegis-proxy/  axum server, config, feeds, metrics, request pipeline
  aegis-bench/  Criterion benchmarks (dev-only)
plugins/
  zero-fee-filter/       sample Wasm rule (Rust guest, Solana)
  approve-guard/         sample Wasm rule (Rust guest, EVM approvals)
  program-guard/         sample Wasm rule (block by program id / selector)
  silent-drop/           sample Wasm rule (HTTP 204 silent drop)
  *.wasm                 pre-built plugins
infra/                   Prometheus + Grafana provisioning
demo/
  demo.sh                one script, three modes (story / wallet / external proxy)
```

## Known limitations

These are deliberate, documented tradeoffs — not oversights. Read them before deploying.

- **Fail-open on parse errors.** An undecodable `sendTransaction` is forwarded rather than blocked:
  the downstream node rejects it with the same decoder, and blocking on our decoder's opinion would
  turn a parser-version drift (e.g. a new vote/message variant) into an outage. The risk is that a
  transaction our decoder cannot understand *also* cannot be screened. A parse-failure counter/alert
  is exposed for operators; a strict `block-on-parse-error` mode is on the roadmap.
- **EVM screening covers the sender and the call target, not calldata recipients.** An address that is
  only referenced inside calldata (e.g. `transfer(sanctioned, …)`) is not matched by the static list.
  Solana has no equivalent gap: every referenced account key, including LUT-loaded ones, is checked.
- **LUT resolution adds an outbound `getAccountInfo` on a cache miss**, inside the request path
  (2 s timeout). A burst of unique lookup tables can force many fetches; the cache evicts wholesale at
  10 000 tables rather than LRU. The benchmark figures above measure the static path only.
- **A Wasm rule runs under a per-call fuel budget** (`WASM_FUEL_LIMIT`), so a runaway rule is stopped,
  but rule evaluation is ~1 ms and is *not* part of the sub-µs static SLA.

## Tests

```bash
cargo test --workspace
```

Unit, integration, and property-based tests (proptest). The property tests fuzz both parsers, the list format, and the feed config with arbitrary input, and check invariants: a transaction is blocked iff it touches a listed address, merged feeds form an attributed union, and base58 and base64 decodes agree.

The server is hardened against request-level abuse: a body-size cap (1 MiB), a batch-length cap (100),
panic isolation via `CatchPanicLayer`, and a per-call fuel budget on Wasm rules. All four are covered
by tests.

## Roadmap

- [ ] Strict `block-on-parse-error` mode (see Known limitations)
- [ ] Calldata-aware EVM screening (screen addresses embedded in calldata, not just sender/target)
- [ ] LRU eviction + negative caching for the LUT resolver
- [ ] Address Lookup Table expansion (Geyser/Yellowstone sync) so v0 LUT-resolved accounts are screened
- [ ] Method-level rate limiting and `getProgramAccounts` cost guards
- [ ] Staked connection pass-through (SWQoS / Jito forwarding)
- [ ] Signed rejection receipts for audit trails
- [ ] Fleet orchestration: push rules and feeds across many nodes

## License

MIT — see [LICENSE](LICENSE).

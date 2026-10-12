# Aegis — Demo Runbook

Everything a judge (or you, on stage) needs to reproduce the demo in under two minutes.

## Which script do I run?

There are five entry points. They share the same proxy binary; they differ in **what they
prove** and **what they require**.

| Script | Needs Docker? | Runs against | Proves | Best for |
|---|---|---|---|---|
| `demo/rehearse.sh` | no | any real Solana RPC | the full six-beat pitch, end-to-end, with a pass/fail gate | **the recording** — one take, everything |
| `demo/wallet-demo.sh` | no | any real Solana/EVM RPC | a **real wallet** (Phantom/MetaMask) blocked at the edge | **the most convincing demo** — you click Send |
| `demo/spam-demo.sh` | no | any real Solana RPC | a spam **burst** blocked at the proxy; legit traffic untouched | showing **volume reduction** (the spam story) |
| `demo/rpc-demo.sh` | no | any real Solana RPC | basic block + live reads + latency histogram | a quick smoke test of the live path |
| `demo/demo.sh` | yes (local stack) | the Compose validator | the same checks against a self-contained stack + Grafana counters | offline/self-contained walkthrough |

None of them need an API key — they default to the public testnet/devnet. Set `HELIUS_API_KEY`
or `AEGIS_RPC_URL` to point them at Helius / mainnet.

### `rehearse.sh` vs `spam-demo.sh` vs `rpc-demo.sh` vs `demo.sh`

- **`rehearse.sh`** is the *story*: six beats (latency → live reads → static block → SIGHUP
  hot-reload → Solana Wasm rule → cross-chain rule hot-swap). It exits non-zero if any beat
  fails, so it is a safe pre-stage check.
- **`wallet-demo.sh`** is the *proof*: it boots Aegis as the wallet's RPC endpoint and you send
  a transaction from **Phantom or MetaMask**. Aegis rejects the blocklisted recipient at the
  edge and the wallet shows the failure — no scripting, an actual click. See the next section.
- **`spam-demo.sh`** is the *volume* argument: it fires a burst of high-compute/zero-fee
  transactions and shows **how many never left the box**, plus the compute the node was never
  asked to reserve. Spam is a volume problem; this is the one script that shows it as volume.
- **`rpc-demo.sh`** is the *smoke test*: a handful of reads, one block, one latency readout.
  Fastest way to confirm the live path works.
- **`demo.sh`** is the *offline* variant: it targets the Docker Compose stack (Aegis + a local
  validator + Prometheus + Grafana), so it also drives the dashboard counters. The other three
  do not touch Docker.

All four build the release binary automatically if it is missing.

---

## Recommended path (no Docker, 60 seconds)

```bash
cargo build --release -p aegis-proxy     # once
bash demo/rehearse.sh                    # the full pitch, six beats
```

## One-take rehearsal (stage recording)

```bash
bash demo/rehearse.sh            # full six-beat run; cites committed latency numbers
BENCH=1 bash demo/rehearse.sh    # also re-run Criterion live (opt-in; idle machine)
INTERACTIVE=1 bash demo/rehearse.sh  # pause between beats for narration
HELIUS_API_KEY=xxx bash demo/rehearse.sh   # run against Helius instead of public testnet
```

> **Criterion is a dev-only tool.** It lives in `[dev-dependencies]` of `aegis-bench` and is
> never compiled into `aegis-proxy` or the Docker image (verify: `cargo tree -p aegis-proxy`).
> The demo cites the committed numbers by default; live re-benchmarking is opt-in with
> `BENCH=1` because a bench is slow and contention-sensitive (on a loaded box Criterion prints
> a spurious "regression").

It runs all six beats against a real cluster and prints a pass/fail summary. Exit code is
0 only if every beat verified — safe to run right before you go on stage.

### The six beats it presents

1. **Latency** — committed Criterion numbers: full pipeline ~700 ns, 50k-entry list ~18 ns (O(1)). Re-run live with `BENCH=1`.
2. **Live reads** — real `getVersion` / `getHealth` / `getSlot` through the firewall.
3. **Static list** — a blocklisted signer rejected at the proxy (`-32001`, `rule_id=blocklist:file`).
4. **Hot reload** — the same tx flips allowed→blocked on `SIGHUP`, no restart, no dropped connections.
5. **Wasm rule (Solana)** — 500k-CU zero-fee spam dropped (`-32002`, `rule_id=zero-fee-spam`).
6. **Cross-chain rule** — hot-swap to the EVM guard; `approve()` blocked (`approve-guard`), `transfer()` passes.

---

## Demo with a real wallet (Phantom / MetaMask)

This is the most convincing demo, and it is **manual by nature** — you click Send, Aegis
blocks it.

**The architecture point.** A wallet broadcasts through the RPC configured for its network —
not through the dApp's provider. So "Aegis in the middle" means **Aegis *is* the wallet's RPC
endpoint**. You are not intercepting a click; you are replacing the node URL, exactly as you
would put a WAF in front of a web origin. Both wallets support a custom RPC per network.

```bash
# Solana devnet / Phantom (build once):
cargo build --release -p aegis-proxy
BLOCK_ADDR=CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv bash demo/wallet-demo.sh

# Base Sepolia / MetaMask:
CHAIN=base BLOCK_ADDR=0x000000000000000000000000000000000000dEaD bash demo/wallet-demo.sh

# Wallets often require https:// — expose via a cloudflared quick tunnel:
TUNNEL=1 BLOCK_ADDR=<addr> bash demo/wallet-demo.sh
```

The script prints the exact RPC URL and the address to send to, then watches `/metrics` live
and logs each block as it happens.

- **Phantom:** Settings → Developer Settings → enable testnet → set Solana Devnet RPC URL to
  the printed URL.
- **MetaMask:** Settings → Networks → *Base Sepolia* → Edit → RPC URL = the printed URL.

Then send a tiny amount **to the printed blocklist address**: Aegis rejects it at the edge
(`-32001`, `rule_id=blocklist:file`) and the wallet surfaces the failure; the transaction
never reaches the chain. Send to any normal address and it passes through untouched.

> **Two ways to insert Aegis:**
> 1. **Wallet RPC** (above) — set the wallet's RPC to Aegis. Works for Phantom *and* MetaMask.
> 2. **dApp-owned provider** — a page that creates its own connection to Aegis
>    (`new Connection(aegis_url)` on Solana, `ethers` provider on EVM). Works cleanly for
>    Solana; for EVM MetaMask still broadcasts through its *network* RPC, so you must also
>    change the network RPC for that path to be enforced.
>
> Note MetaMask shows a *gas-estimation* error rather than the raw JSON-RPC text, because it
> estimates gas via the same RPC before sending and Aegis refuses the request. The rejection
> is still Aegis; watch the terminal log to see it fire.

### localhost is fine — no tunnel needed

Pointing Phantom at `http://127.0.0.1:8899` is the normal local-validator workflow and is the
simplest way to record. Only reach for `TUNNEL=1` if a wallet refuses a non-`https://` URL.

**Phantom for both chains:**

- **Solana Devnet** — Settings → Developer Settings → enable testnet → set the Solana Devnet
  RPC URL to `http://127.0.0.1:8899`. Send a transfer to the printed blocklist address.
- **EVM (Base Sepolia)** — Phantom's EVM networks work the same way *if* your version lets you
  edit the network's RPC URL; set it to `http://127.0.0.1:8899`. If Phantom won't let you
  override an EVM RPC, use MetaMask for the EVM half (it accepts `http://localhost` custom
  RPCs), or demo EVM via `demo/rehearse.sh` Beat 6.

One run only fronts **one** upstream chain, so pick the network to match the script:

```bash
# Phantom → Solana Devnet
BLOCK_ADDR=CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv bash demo/wallet-demo.sh

# Phantom → Base Sepolia (EVM)
CHAIN=base BLOCK_ADDR=0x000000000000000000000000000000000000dEaD bash demo/wallet-demo.sh
```

> Note: Aegis rejections are identified by `error.data.rule_id` (present on its own
> rejections, absent on downstream node errors). JSON-RPC codes alone can collide with the
> chain — e.g. Solana itself returns `-32002` for a failed transaction simulation.

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

## Path C — Docker against a real RPC (Grafana included)

Aegis + Prometheus + Grafana in front of any external RPC — **no emulated validator**, so
this works cleanly on Apple silicon.

```bash
docker compose -f docker-compose.rpc.yml up --build
# Helius / devnet:
AEGIS_RPC_URL="https://testnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" \
  docker compose -f docker-compose.rpc.yml up --build
```

Ports: Aegis `:8899`, Prometheus `:9090`, **Grafana `:3001`** (`http://localhost:3001`).

Drive the dashboard — run a burst so the counters move, then open Grafana:

```bash
SPAM=500 bash demo/spam-demo.sh          # 500 blocks show up in "Blocked, cumulative"
open http://localhost:3001               # dashboard uid: aegis-svm-v1
```

> **Which paths have Grafana?** Only the two Docker stacks (Path B and Path C). The
> no-Docker scripts (`rehearse.sh`, `spam-demo.sh`, `rpc-demo.sh`) run the bare binary: they
> still *serve* `/metrics`, but nothing scrapes it and the proxy exits when the script ends —
> so **no Grafana**. If you want the dashboard, use a Compose path.

---

## The 90-second stage script

1. **Frame the problem (15s).** "Solana settles in ~400 ms slots. Existing screening is
   either a 150 ms REST round-trip in the hot path, or an HTTP WAF that never sees inside
   the transaction. So operators carry traffic they don't want — spam, drainers, exploits,
   sanctioned counterparties."

2. **Show the number (15s).** Cite the committed figure: full pipeline **~700 ns** on Solana
   (parse + policy), 50 000-entry list still **~18 ns** (O(1)). That is ~70 000× under a
   50 µs budget. (Reproduce live with `BENCH=1 bash demo/rehearse.sh` on an idle machine.)

3. **Show the live block (30s).** `bash demo/rpc-demo.sh` — a blocklisted signer is
   rejected at the proxy (`-32001`, `rule_id=blocklist:file`) while real cluster reads
   pass through untouched. For the **volume** story, `bash demo/spam-demo.sh` shows N spam
   txs blocked, 0 forwarded, and the compute the node never reserved.

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

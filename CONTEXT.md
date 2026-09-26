# Aegis-SVM Domain Glossary

## Terms

**Inbound Request** — A JSON-RPC HTTP request arriving at Aegis's listening port from a client (wallet, bot, dApp).

**Transaction** — A Solana `VersionedTransaction` (v0 or legacy) encoded as base64 within a `sendTransaction` JSON-RPC call. The primary unit of inspection.

**Inspection Pipeline** — The ordered sequence of stages (Parser → Static Engine → Wasm Engine) that evaluates a Transaction before it is forwarded or rejected. Runs synchronously on the hot path.

**Parser Stage** — Stage 1 of the Inspection Pipeline. Deserializes the raw Transaction bytes using `solana-sdk` and extracts: static account keys (Signers, Writable Accounts, Program IDs) plus best-effort LUT-expanded addresses.

**Static Engine** — Stage 2. Checks each extracted pubkey from the Parser Stage against the in-memory Blocklist. O(1) per lookup.

**Wasm Engine** — Stage 3. Evaluates the Transaction against operator-defined Rules loaded as Extism Wasm plugins. Hot-reloadable. Stretch goal for the hackathon.

**Blocklist** — An `AHashSet<Pubkey>` loaded from a plaintext file (one base58 pubkey per line) at startup. Reloaded on SIGHUP without dropping connections.

**Rule** — An operator-defined policy expressed as YAML config referencing an Extism Wasm plugin. Evaluated by the Wasm Engine. Returns a Verdict.

**Verdict** — The output of any pipeline stage: `Allow`, `Block { rule_id, rejection_code, message }`, or `Drop` (silent discard, no response).

**Downstream** — The target RPC node (solana-test-validator or production validator) that receives Allowed transactions after the Inspection Pipeline passes them.

**Pass-through** — Any JSON-RPC method other than `sendTransaction`. Forwarded to Downstream without entering the Inspection Pipeline.

**LUT Cache** — An in-memory map of Address Lookup Table pubkey → account list, used by the Parser Stage for best-effort expansion of v0 transaction addresses. A cache miss results in the transaction proceeding with only static keys inspected.

**Rejection Response** — A structured JSON-RPC error returned synchronously to the caller when a Transaction is Blocked. Contains `code`, `message`, `rule_id`, and `trace_id`.

**Metrics** — Prometheus counters and histograms exposed at `/metrics`. Tracks transactions inspected, blocked (by rule), and Inspection Pipeline latency in microseconds.

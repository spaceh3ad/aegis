# Wayfinder Map: Aegis-SVM Implementation Plan

**Label:** `wayfinder:map`

## Destination

A fully-specified, task-level implementation plan for Aegis-SVM ready to execute: Cargo workspace scaffolded, all crate interfaces defined, Docker Compose topology locked, and the demo script specified — enough for a developer to build without further design decisions.

## Notes

- Domain: Rust reverse proxy for Solana JSON-RPC, sub-50µs latency target
- Consult CONTEXT.md for canonical terms
- Skills per session: grilling, domain-modeling, research
- Stack: Tokio + axum + solana-sdk + ahash + prometheus + extism (Wasm, stretch)
- Demo: Docker Compose (aegis + solana-test-validator + prometheus + grafana), one-command startup
- Blocklist: plaintext file, one base58 pubkey/line, reload on SIGHUP
- LUT: best-effort, pass through on cache miss

## Decisions so far

- [Async runtime is Tokio](tickets/TICKET-001-async-runtime.md): Tokio chosen; no alternatives evaluated.
- [HTTP server is axum](tickets/TICKET-002-http-server.md): axum for routing + middleware; sendTransaction intercepted, all else pass-through.
- [Transaction deserialization uses solana-sdk](tickets/TICKET-003-tx-deserialization.md): Full solana-sdk VersionedTransaction; zero-copy optimization deferred.
- [LUT resolution is best-effort](tickets/TICKET-004-lut-resolution.md): Attempt expansion; pass through on cache miss. No Geyser dependency for hackathon.
- [Blocklist reload via SIGHUP](tickets/TICKET-005-blocklist-reload.md): SIGHUP triggers atomic swap of AHashSet; no file-watcher dependency.
- [Observability is Prometheus + Grafana](tickets/TICKET-006-observability.md): /metrics endpoint + full stack in Docker Compose.
- [Cargo workspace with three crates](tickets/TICKET-007-workspace.md): aegis-core, aegis-proxy, aegis-bench.
- [Wasm engine uses Extism](tickets/TICKET-008-wasm-engine.md): Extism SDK; stretch goal, built last.
- [Non-sendTransaction methods are transparent pass-through](tickets/TICKET-009-passthrough.md): No inspection, no filtering.
- [solana-sdk deserialization API shape](tickets/TICKET-010-sdk-api-shape.md): base64 → bincode → VersionedTransaction; signers via header.num_required_signers; LUTs via address_table_lookups.
- [axum transparent proxy pattern](tickets/TICKET-011-axum-proxy.md): Buffer body as Bytes, inspect JSON-RPC method, reject or forward via pooled reqwest::Client.
- [Extism Rust SDK invocation](tickets/TICKET-012-extism-api.md): extism 1.13+; plugin panics don't crash host; **Wasm adds ~1ms per call — not part of sub-50µs claim**.
- [Prometheus metrics schema](tickets/TICKET-013-metrics-schema.md): 5 metrics — transactions_total, blocked_total, inspection_duration_microseconds histogram, blocklist_size gauge, downstream_up gauge.
- [Docker Compose service topology](tickets/TICKET-014-compose-topology.md): aegis:8899+9090, validator:8900 (internal), prometheus:9091, grafana:3000.

## Open Tickets (Frontier)

None — map complete. See implementation plan.

## Not yet specified

- Wasm plugin input/output contract (what data a Rule receives, what it returns) — depends on TICKET-010 and TICKET-012 resolving first
- Criterion benchmark harness structure — depends on aegis-core interface being locked

## Out of scope

- Geyser/Yellowstone stream integration for LUT sync (production feature, not hackathon)
- High-availability clustering / distributed rate limiting (Enterprise Edition)
- Chainalysis/TRM direct feed integration
- sendVersionedTransaction separate handling (treat identically to sendTransaction)

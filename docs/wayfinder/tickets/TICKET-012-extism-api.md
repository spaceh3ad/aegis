# TICKET-012: Extism Rust SDK invocation pattern

**Type:** `wayfinder:research`
**Status:** closed

## Resolution

**Crate:** `extism` 1.13.0+ (host), `extism-pdk` (guest/Wasm side).

```rust
// Load plugin
let manifest = Manifest::new([Wasm::file("plugin.wasm")]);
let mut plugin = Plugin::new(&manifest, [], true)?;

// Call with JSON
let result = plugin.call::<String, Json<Value>>("analyze", input_json)?;

// Hot-swap (reloads plugin, old state lost)
plugin.update(&new_manifest)?;
```

**Safety:** Plugin panics do NOT crash host — Wasmtime sandbox isolates memory. Returns `Err` on panic.

**CRITICAL — Latency:** Each plugin call costs **0.5–2ms** on modern CPUs. This means the Wasm Engine **cannot contribute to the sub-50µs pipeline claim**. Stages 1+2 (Parser + Static Engine) achieve sub-50µs. Stage 3 adds ~1ms. Demo and benchmarks must be clear about this distinction.

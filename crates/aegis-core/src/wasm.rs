use std::path::Path;
use std::sync::{Arc, Mutex};

use extism::{Manifest, Plugin, Wasm};
use serde::{Deserialize, Serialize};

use crate::types::{InspectedTx, Verdict};

#[derive(Debug, thiserror::Error)]
pub enum WasmEngineError {
    #[error("failed to load wasm plugin: {0}")]
    Load(#[from] extism::Error),
    #[error("plugin call failed: {0}")]
    Call(String),
    #[error("invalid plugin output: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Serialised view of an inspected transaction sent into the Wasm guest.
#[derive(Serialize)]
struct PluginInput<'a> {
    signers:       Vec<&'a str>,
    programs:      Vec<&'a str>,
    cu_requested:  u64,
    priority_fee:  u64,
}

#[derive(Deserialize)]
struct PluginOutput {
    verdict:  String,
    rule_id:  Option<String>,
    message:  Option<String>,
}

/// Hot-reloadable Extism plugin wrapper.
///
/// The plugin is accessed behind a Mutex so SIGHUP can swap it atomically.
/// The overhead is ~1ms per call (wasmtime JIT cost); callers must account
/// for this being outside the sub-50µs static-engine SLA.
#[derive(Clone)]
pub struct WasmEngine {
    plugin: Arc<Mutex<Plugin>>,
}

impl WasmEngine {
    pub fn load(path: &Path) -> Result<Self, WasmEngineError> {
        let plugin = Self::make_plugin(path)?;
        Ok(Self {
            plugin: Arc::new(Mutex::new(plugin)),
        })
    }

    pub fn reload(&self, path: &Path) -> Result<(), WasmEngineError> {
        let new_plugin = Self::make_plugin(path)?;
        *self.plugin.lock().expect("wasm plugin lock poisoned") = new_plugin;
        Ok(())
    }

    fn make_plugin(path: &Path) -> Result<Plugin, extism::Error> {
        let wasm = Wasm::file(path);
        let manifest = Manifest::new([wasm]);
        Plugin::new(&manifest, [], true)
    }

    pub fn evaluate(&self, tx: &InspectedTx, cu_requested: u64, priority_fee: u64) -> Option<Verdict> {
        let signers: Vec<String> = tx.signers.iter().map(bs58::encode).map(|e| e.into_string()).collect();
        let programs: Vec<String> = tx.program_ids.iter().map(bs58::encode).map(|e| e.into_string()).collect();

        let input = PluginInput {
            signers:      signers.iter().map(String::as_str).collect(),
            programs:     programs.iter().map(String::as_str).collect(),
            cu_requested,
            priority_fee,
        };

        let input_json = serde_json::to_vec(&input).ok()?;

        let output_bytes: Vec<u8> = {
            let mut plugin = self.plugin.lock().expect("wasm plugin lock poisoned");
            plugin.call::<&[u8], &[u8]>("evaluate", &input_json).ok()?.to_vec()
        };

        let output: PluginOutput = serde_json::from_slice(&output_bytes).ok()?;

        match output.verdict.as_str() {
            "block" => Some(Verdict::Block {
                rule_id: output.rule_id.unwrap_or_else(|| "wasm-rule".into()),
                code: -32002,
                message: output.message.unwrap_or_else(|| "Blocked by Wasm rule".into()),
            }),
            "drop" => Some(Verdict::Drop),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn plugin_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm")
    }

    #[test]
    fn test_wasm_engine_zero_fee_block() {
        let path = plugin_path();
        if !path.exists() {
            eprintln!("SKIP: zero-fee-filter.wasm not built — run `cargo build --release --target wasm32-wasip1 -p zero-fee-filter` first");
            return;
        }

        let engine = WasmEngine::load(&path).expect("load plugin");
        let tx = InspectedTx {
            static_keys: vec![],
            signers: vec![[0u8; 32]],
            writable: vec![],
            program_ids: vec![[0u8; 32]],
            lut_refs: vec![],
        };

        let verdict = engine.evaluate(&tx, 500_000, 0);
        assert!(
            matches!(verdict, Some(Verdict::Block { .. })),
            "500k CU with 0 priority fee should be blocked"
        );

        let verdict_ok = engine.evaluate(&tx, 500_000, 1000);
        assert!(
            verdict_ok.is_none(),
            "500k CU with priority fee should pass"
        );
    }
}

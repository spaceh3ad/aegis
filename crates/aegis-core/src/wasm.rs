use std::path::Path;
use std::sync::{Arc, Mutex};

use extism::{Manifest, Plugin, Wasm};
use serde::{Deserialize, Serialize};

use crate::types::{EvmTx, InspectedTx, Verdict};

pub const WASM_RULE_CODE: i32 = -32002;

#[derive(Debug, thiserror::Error)]
pub enum WasmEngineError {
    #[error("failed to load wasm plugin: {0}")]
    Load(#[from] extism::Error),
    #[error("plugin call failed: {0}")]
    Call(String),
    #[error("invalid plugin output: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Chain-agnostic JSON view handed to the Wasm guest.
///
/// Solana: `signers` are fee payer + co-signers, `programs` are invoked program ids,
/// `cu_requested` / `priority_fee` come from ComputeBudget instructions (0 when absent).
/// EVM: `signers` is the recovered sender, `programs` the call target (empty on create),
/// `priority_fee` is the tip (or gas price for legacy), `cu_requested` is the gas limit.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuleInput {
    pub chain: &'static str,
    pub signers: Vec<String>,
    pub programs: Vec<String>,
    pub cu_requested: u64,
    pub priority_fee: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_wei: Option<String>,
}

impl From<&InspectedTx> for RuleInput {
    fn from(tx: &InspectedTx) -> Self {
        let b58 = |k: &[u8; 32]| bs58::encode(k).into_string();
        Self {
            chain: "svm",
            signers: tx.signers.iter().map(b58).collect(),
            programs: tx.program_ids.iter().map(b58).collect(),
            cu_requested: tx.cu_limit.unwrap_or(0) as u64,
            priority_fee: tx.cu_price.unwrap_or(0),
            chain_id: None,
            selector: None,
            value_wei: None,
        }
    }
}

impl From<&EvmTx> for RuleInput {
    fn from(tx: &EvmTx) -> Self {
        let h = |a: &[u8; 20]| format!("0x{}", hex::encode(a));
        Self {
            chain: "evm",
            signers: vec![h(&tx.from)],
            programs: tx.to.iter().map(h).collect(),
            cu_requested: tx.gas_limit,
            priority_fee: u64::try_from(tx.priority_fee).unwrap_or(u64::MAX),
            chain_id: tx.chain_id,
            selector: tx.selector.map(|s| format!("0x{}", hex::encode(s))),
            value_wei: Some(tx.value_wei.clone()),
        }
    }
}

#[derive(Deserialize)]
struct PluginOutput {
    verdict: String,
    rule_id: Option<String>,
    message: Option<String>,
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

    /// Fails open: a plugin error or unparseable output yields `None` (no opinion).
    pub fn evaluate(&self, input: &RuleInput) -> Option<Verdict> {
        let input_json = serde_json::to_vec(input).ok()?;

        let output_bytes: Vec<u8> = {
            let mut plugin = self.plugin.lock().expect("wasm plugin lock poisoned");
            plugin
                .call::<&[u8], &[u8]>("evaluate", &input_json)
                .ok()?
                .to_vec()
        };

        let output: PluginOutput = serde_json::from_slice(&output_bytes).ok()?;

        match output.verdict.as_str() {
            "block" => Some(Verdict::Block {
                rule_id: output.rule_id.unwrap_or_else(|| "wasm-rule".into()),
                code: WASM_RULE_CODE,
                message: output
                    .message
                    .unwrap_or_else(|| "Blocked by Wasm rule".into()),
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

    fn svm_tx(cu_limit: Option<u32>, cu_price: Option<u64>) -> InspectedTx {
        InspectedTx {
            signers: vec![[0u8; 32]],
            program_ids: vec![[0u8; 32]],
            cu_limit,
            cu_price,
            ..Default::default()
        }
    }

    #[test]
    fn test_rule_input_from_svm() {
        let input = RuleInput::from(&svm_tx(Some(500_000), Some(7)));
        assert_eq!(input.chain, "svm");
        assert_eq!(input.cu_requested, 500_000);
        assert_eq!(input.priority_fee, 7);
        assert_eq!(input.signers, vec!["11111111111111111111111111111111"]);
        let json = serde_json::to_value(&input).unwrap();
        assert!(
            json.get("chain_id").is_none(),
            "EVM-only fields omitted for svm"
        );
    }

    #[test]
    fn test_rule_input_from_evm() {
        let tx = EvmTx {
            chain_id: Some(42161),
            from: [0xaa; 20],
            to: Some([0xbb; 20]),
            selector: Some([0x09, 0x5e, 0xa7, 0xb3]),
            gas_limit: 50_000,
            priority_fee: u128::MAX,
            value_wei: "5".into(),
            tx_type: 2,
        };
        let input = RuleInput::from(&tx);
        assert_eq!(input.chain, "evm");
        assert_eq!(input.signers, vec![format!("0x{}", "aa".repeat(20))]);
        assert_eq!(input.programs, vec![format!("0x{}", "bb".repeat(20))]);
        assert_eq!(input.selector.as_deref(), Some("0x095ea7b3"));
        assert_eq!(
            input.priority_fee,
            u64::MAX,
            "saturates instead of wrapping"
        );
        assert_eq!(input.chain_id, Some(42161));
    }

    #[test]
    fn test_wasm_engine_zero_fee_block() {
        let path = plugin_path();
        if !path.exists() {
            eprintln!("SKIP: zero-fee-filter.wasm not built, run `cargo build --release --target wasm32-wasip1` in plugins/zero-fee-filter first");
            return;
        }

        let engine = WasmEngine::load(&path).expect("load plugin");

        let spam = RuleInput::from(&svm_tx(Some(500_000), None));
        assert!(
            matches!(
                engine.evaluate(&spam),
                Some(Verdict::Block {
                    code: WASM_RULE_CODE,
                    ..
                })
            ),
            "500k CU with 0 priority fee should be blocked"
        );

        let paid = RuleInput::from(&svm_tx(Some(500_000), Some(1_000)));
        assert!(
            engine.evaluate(&paid).is_none(),
            "500k CU with priority fee should pass"
        );

        let mut evm = spam.clone();
        evm.chain = "evm";
        assert!(
            engine.evaluate(&evm).is_none(),
            "rule is scoped to svm; high EVM gas limit is normal"
        );
    }
}

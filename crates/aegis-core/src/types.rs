use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct InspectedTx {
    pub static_keys: Vec<[u8; 32]>,
    pub signers: Vec<[u8; 32]>,
    pub writable: Vec<[u8; 32]>,
    pub program_ids: Vec<[u8; 32]>,
    pub lut_refs: Vec<[u8; 32]>,
    /// From a ComputeBudget SetComputeUnitLimit instruction, if present.
    pub cu_limit: Option<u32>,
    /// From a ComputeBudget SetComputeUnitPrice instruction (micro-lamports per CU), if present.
    pub cu_price: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmTx {
    pub chain_id: Option<u64>,
    /// Recovered from the signature.
    pub from: [u8; 20],
    /// `None` for contract creation.
    pub to: Option<[u8; 20]>,
    pub selector: Option<[u8; 4]>,
    pub gas_limit: u64,
    /// Priority fee for EIP-1559 style txs, gas price for legacy/2930.
    pub priority_fee: u128,
    /// Decimal string: U256 does not fit in JSON numbers.
    pub value_wei: String,
    pub tx_type: u8,
}

#[derive(Debug, Clone)]
pub enum Verdict {
    Allow,
    Block { rule_id: String, code: i32, message: String },
    Drop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionResponse {
    pub jsonrpc: String,
    pub error: RejectionError,
    pub id: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionError {
    pub code: i32,
    pub message: String,
    pub data: RejectionData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionData {
    pub rule_id: String,
    pub trace_id: String,
}

impl RejectionResponse {
    pub fn new(
        id: serde_json::Value,
        code: i32,
        message: impl Into<String>,
        rule_id: impl Into<String>,
        trace_id: impl Into<String>,
    ) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            error: RejectionError {
                code,
                message: message.into(),
                data: RejectionData {
                    rule_id: rule_id.into(),
                    trace_id: trace_id.into(),
                },
            },
            id,
        }
    }
}

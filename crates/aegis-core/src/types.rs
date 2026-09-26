use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct InspectedTx {
    pub static_keys: Vec<[u8; 32]>,
    pub signers: Vec<[u8; 32]>,
    pub writable: Vec<[u8; 32]>,
    pub program_ids: Vec<[u8; 32]>,
    pub lut_refs: Vec<[u8; 32]>,
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

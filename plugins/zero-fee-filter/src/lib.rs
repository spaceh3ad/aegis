use extism_pdk::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Input {
    #[serde(default = "default_chain")]
    chain: String,
    #[serde(default)]
    cu_requested: u64,
    #[serde(default)]
    priority_fee: u64,
}

fn default_chain() -> String {
    "svm".into()
}

#[derive(Serialize)]
struct Output {
    verdict:  &'static str,
    rule_id:  &'static str,
    message:  &'static str,
}

#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let input = input.0;
    if input.chain == "svm" && input.cu_requested > 400_000 && input.priority_fee == 0 {
        Ok(Json(Output {
            verdict: "block",
            rule_id: "zero-fee-spam",
            message: "High-CU transaction with zero priority fee, likely spam",
        }))
    } else {
        Ok(Json(Output {
            verdict: "allow",
            rule_id: "",
            message: "",
        }))
    }
}

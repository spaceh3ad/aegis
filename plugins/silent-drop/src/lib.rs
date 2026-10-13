use extism_pdk::*;
use serde::{Deserialize, Serialize};

/// A program to shadow-drop: SVM program id, and an EVM selector.
const DROP_SVM: &[&str] = &["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"];
const DROP_EVM_SELECTOR: &[&str] = &["0xa9059cbb"]; // ERC-20 transfer(address,uint256)

#[derive(Deserialize)]
struct Input {
    #[serde(default = "default_chain")]
    chain: String,
    #[serde(default)]
    programs: Vec<String>,
    #[serde(default)]
    selector: Option<String>,
}

fn default_chain() -> String {
    "svm".into()
}

#[derive(Serialize)]
struct Output {
    verdict: &'static str,
    rule_id: &'static str,
    message: &'static str,
}

/// Returns `drop` (HTTP 204, no response) instead of a visible rejection, so a prober cannot
/// tell the firewall is there or learn which rule matched. Anti-reconnaissance posture.
#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let input = input.0;
    let hit = if input.chain == "evm" {
        input
            .selector
            .as_deref()
            .is_some_and(|s| DROP_EVM_SELECTOR.iter().any(|d| d.eq_ignore_ascii_case(s)))
    } else {
        input.programs.iter().any(|p| DROP_SVM.iter().any(|d| *d == p))
    };

    Ok(Json(if hit {
        Output { verdict: "drop", rule_id: "silent-drop", message: "shadow-dropped" }
    } else {
        Output { verdict: "allow", rule_id: "", message: "" }
    }))
}

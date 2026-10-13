use extism_pdk::*;
use serde::{Deserialize, Serialize};

// Demo denylists. A real operator supplies these; Aegis ships none.
/// SPL Token program — blocking it refuses every SPL token instruction.
const DENY_SVM: &[&str] = &["TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"];
/// ERC-20 `transferFrom(address,address,uint256)` — a common drainer primitive.
const DENY_EVM_SELECTOR: &[&str] = &["0x23b872dd"];

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

/// Blocks by *what the transaction calls*, not just who signs it.
/// SVM: any invoked program id in the denylist. EVM: selector in the denylist.
#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let input = input.0;
    let hit = if input.chain == "evm" {
        input
            .selector
            .as_deref()
            .is_some_and(|s| DENY_EVM_SELECTOR.iter().any(|d| d.eq_ignore_ascii_case(s)))
    } else {
        input
            .programs
            .iter()
            .any(|p| DENY_SVM.iter().any(|d| *d == p))
    };

    Ok(Json(if hit {
        Output {
            verdict: "block",
            rule_id: "program-guard",
            message: "Transaction invokes a program/selector on the operator denylist",
        }
    } else {
        Output { verdict: "allow", rule_id: "", message: "" }
    }))
}

use extism_pdk::*;
use serde::{Deserialize, Serialize};

/// ERC-20 `approve(address,uint256)` selector.
const APPROVE_SELECTOR: &str = "0x095ea7b3";
/// ERC-20 `increaseAllowance(address,uint256)` selector — same drainer vector.
const INCREASE_ALLOWANCE_SELECTOR: &str = "0x39509351";

#[derive(Deserialize)]
struct Input {
    #[serde(default = "default_chain")]
    chain: String,
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

/// Blocks ERC-20 approval calls on EVM. A malicious approval is the entry point for
/// most token drainers, so the operator may refuse them at the RPC boundary outright.
#[plugin_fn]
pub fn evaluate(input: Json<Input>) -> FnResult<Json<Output>> {
    let input = input.0;
    let selector = input.selector.as_deref();
    let blocked = input.chain == "evm"
        && matches!(selector, Some(APPROVE_SELECTOR) | Some(INCREASE_ALLOWANCE_SELECTOR));

    Ok(Json(if blocked {
        Output {
            verdict: "block",
            rule_id: "approve-guard",
            message: "ERC-20 approval call blocked by policy",
        }
    } else {
        Output {
            verdict: "allow",
            rule_id: "",
            message: "",
        }
    }))
}

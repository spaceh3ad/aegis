#!/usr/bin/env bash
# CASE 3 — EVM approval (drainer entry point).
#   approve()/increaseAllowance() are how drainers get spending rights. Aegis refuses them at
#   the edge. The token is a real ERC-20 deployed to anvil (see setup.sh).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

TOKEN=$(art erc20.txt)
[ -n "$TOKEN" ] || { warn "run demo/cases/setup.sh first"; exit 1; }
SPENDER="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"   # anvil account #1
TO="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"

step "0. Load the approval guard (Wasm)"
set_rule approve-guard
note "rule: EVM selector 0x095ea7b3 (approve) or 0x39509351 (increaseAllowance)  -> block"

step "1. A plain transfer — allowed"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" transfer "$TOKEN" "$TO" 1 "$(evm_nonce)")")
expect_allow "$code" "$rule" "ERC-20 transfer()"
[ -n "$res" ] && note "anvil accepted it, hash $res…"

step "2. approve(spender, MAX_UINT) — the drainer primitive, blocked"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")")
expect_block "$code" "$rule" "ERC-20 approve(address,uint256.max)"

step "3. increaseAllowance — same vector, blocked"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" raw "$TOKEN" 39509351 "$(evm_nonce)")")
expect_block "$code" "$rule" "ERC-20 increaseAllowance(address,uint256)"

step "Done — terminal 2 shows selector=0x095ea7b3 blocked with rule=approve-guard."

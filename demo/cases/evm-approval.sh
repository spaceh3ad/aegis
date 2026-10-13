#!/usr/bin/env bash
# CASE 3 — EVM approval (drainer entry point).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
TOKEN=$(art erc20.txt); SPENDER="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"; TO="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"

say "load approval guard"
set_rule approve-guard

say "transfer() — allowed"
read -r code rule < <(send_evm "$(python3 "$CASES/evmtx.py" transfer "$TOKEN" "$TO" 1 "$(evm_nonce)")"); report_allow "$rule"

say "approve(spender, MAX_UINT) — blocked"
read -r code rule < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")"); report_block "$rule"

say "increaseAllowance() — blocked"
read -r code rule < <(send_evm "$(python3 "$CASES/evmtx.py" raw "$TOKEN" 39509351 "$(evm_nonce)")"); report_block "$rule"

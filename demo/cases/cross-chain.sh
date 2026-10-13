#!/usr/bin/env bash
# CASE 4 — cross-chain rule hot-swap (one binary, swap the live policy).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
TOKEN=$(art erc20.txt); SPENDER="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

say "both proxies load the Solana spam rule"
set_rule zero-fee-filter

say "solana spam — blocked"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)"); report_block "$rule"

say "evm approve — not yet blocked (rule is Solana-scoped)"
read -r code rule < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")"); report_allow "$rule"

say "hot-swap the rule to approve-guard + SIGHUP (no restart)"
set_rule approve-guard; note "SIGHUP sent"

say "evm approve — now blocked, same running process"
read -r code rule < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")"); report_block "$rule"

say "solana spam — now ignored (policy is per-deployment)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)"); report_allow "$rule"

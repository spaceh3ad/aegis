#!/usr/bin/env bash
# CASE 6 — block by program / selector (refuse an action, not just an address).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
TOKEN=$(art erc20.txt); PAY=$(art payprog.txt); TO="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"

say "load program-guard (SVM: SPL Token program; EVM: selector 0x23b872dd)"
set_rule program-guard

say "evm transfer() — allowed"
read -r code rule < <(send_evm "$("$PY" "$CASES/evmtx.py" transfer "$TOKEN" "$TO" 1 "$(evm_nonce)")" "transfer()  selector a9059cbb"); report_allow "$rule"

say "evm transferFrom() — blocked by selector"
read -r code rule < <(send_evm "$("$PY" "$CASES/evmtx.py" transferfrom "$TOKEN" "$EVM_FROM" "$TO" 1 "$(evm_nonce)")" "transferFrom()  selector 23b872dd"); report_block "$rule"

say "svm: invoking the SPL Token program — blocked"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)" "invokes SPL Token program"); report_block "$rule"

say "svm: invoking your own program — allowed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" program "$PAY")" "invokes $PAY (unwatched)"); report_allow "$rule"

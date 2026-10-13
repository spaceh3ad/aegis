#!/usr/bin/env bash
# CASE 6 — Block by program / selector.
#   The static list matches addresses. A rule matches *what the transaction does*: the program
#   ids it invokes (SVM) or the method selector it calls (EVM) — regardless of who signs it.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

PAY=$(art payprog.txt)
TOKEN=$(art erc20.txt)
USER=$(art user.pub)
TO="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"

step "0. Load the program-guard rule"
set_rule program-guard
note "SVM denylist: SPL Token program    EVM denylist: selector 0x23b872dd (transferFrom)"

step "1. EVM: transfer() is allowed, but transferFrom() is blocked by selector"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" transfer "$TOKEN" "$TO" 1 "$(evm_nonce)")")
expect_allow "$code" "$rule" "transfer() — selector not on the denylist"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" transferfrom "$TOKEN" "$EVM_FROM" "$TO" 1 "$(evm_nonce)")")
expect_block "$code" "$rule" "transferFrom() — selector 0x23b872dd denied"

step "2. SVM: invoking the SPL Token program is refused; your own program is not"
SPL_TX="$(python3 "$CASES/txbuild.py" program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)"
read -r code rule < <(send_sol "$SPL_TX")
expect_block "$code" "$rule" "tx invoking the SPL Token program"
if [ -n "$PAY" ]; then
  read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" program "$PAY")")
  expect_allow "$code" "$rule" "tx invoking payments-program (not on the denylist)"
fi

step "3. Why this matters"
say "address lists answer 'who'. rule policy answers 'what' — a whole class of action,"
say "refused regardless of signer. Terminal 2 shows target=<program id>/selector=<hex>."

step "Done — the engine is programmable; the two shipped rules are just examples."

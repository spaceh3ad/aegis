#!/usr/bin/env bash
# CASE 4 — Cross-chain rule hot-swap.
#   One Aegis process per chain, same binary. The point: swap the *loaded policy* live via
#   SIGHUP and the same running process changes what it enforces — no restart, no dropped
#   connections. Here: a Solana spam rule becomes an EVM approval guard.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

TOKEN=$(art erc20.txt)
SPENDER="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"

step "0. Both proxies start with the Solana spam rule"
set_rule zero-fee-filter
note "same rule file is watched by both the sol and evm Aegis instances"

step "1. Solana side: zero-fee 500k-CU tx is blocked"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)")
expect_block "$code" "$rule" "Solana spam"

step "2. EVM side: approve() is NOT blocked yet (this rule is Solana-scoped)"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")")
expect_allow "$code" "$rule" "approve() under the Solana rule"

step "3. Hot-swap the rule file to approve-guard + SIGHUP (no restart)"
set_rule approve-guard
note "overwrote rule.wasm and signalled; watch terminal 2: 'wasm plugin reloaded'"

step "4. EVM side: approve() is now blocked — live, same process"
read -r code rule res < <(send_evm "$(python3 "$CASES/evmtx.py" approve "$TOKEN" "$SPENDER" max "$(evm_nonce)")")
expect_block "$code" "$rule" "approve() after hot-swap"

step "5. The Solana side now ignores spam (rule is EVM-scoped) — policy is per-deployment"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)")
expect_allow "$code" "$rule" "spam under the EVM rule"

step "Done — 'one binary, swappable policy, two chains'. Watch terminal 2 for the reload lines."

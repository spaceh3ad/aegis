#!/usr/bin/env bash
# CASE 5 — Silent drop (HTTP 204).
#   Instead of a visible JSON-RPC error, a policy can return 204 No Content: the transaction
#   simply vanishes, with no body and no rule_id. Anti-reconnaissance — a prober cannot tell
#   the firewall is there or which rule fired.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

PAY=$(art payprog.txt)

step "0. Load the silent-drop rule"
set_rule silent-drop
note "rule drops any tx invoking the SPL Token program (SVM) / transfer(address,uint256) (EVM)"

step "1. A transaction that matches — expect HTTP 204, empty body"
# silent-drop denies the SPL Token program, so invoke exactly that.
TX="$(python3 "$CASES/txbuild.py" program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)"
status=$(rpc_status "$PROXY_SOL" sendTransaction "[\"$TX\",{\"encoding\":\"base64\"}]")
expect_drop "$status" "tx invoking the SPL Token program"

step "2. Compare: a non-matching tx is allowed normally (HTTP 200)"
CLEAN=$(python3 "$CASES/txbuild.py" simple "$(python3 -c 'import os;print(os.urandom(32).hex())')")
status=$(rpc_status "$PROXY_SOL" sendTransaction "[\"$CLEAN\",{\"encoding\":\"base64\"}]")
[ "$status" = "200" ] && ok "unmatched tx → HTTP $status (forwarded)" || warn "expected 200, got $status"

step "3. The contrast that matters"
say "block() returns a JSON-RPC error with a rule_id — the client learns why."
say "drop() returns 204 with nothing — the client learns nothing."
note "terminal 2 still logs it internally (DROPPED) so the operator keeps the audit trail."

step "Done — choose block vs drop per rule depending on how visible you want to be."

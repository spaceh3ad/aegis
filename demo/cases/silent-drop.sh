#!/usr/bin/env bash
# CASE 5 — silent drop (HTTP 204).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis

say "load silent-drop rule (denies the SPL Token program)"
set_rule silent-drop

say "tx invoking the SPL Token program — dropped with no body"
TX=$(python3 "$CASES/txbuild.py" program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA)
trace "sendTransaction  invokes SPL Token program (Tokenkeg…VQ5DA)"
status=$(rpc_status "$PROXY_SOL" sendTransaction "[\"$TX\",{\"encoding\":\"base64\"}]"); report_drop "$status"

say "a non-matching tx — forwarded normally (HTTP 200)"
CLEAN=$(python3 "$CASES/txbuild.py" simple "$(python3 -c 'import os;print(os.urandom(32).hex())')")
trace "sendTransaction  plain transfer (no watched program)"
status=$(rpc_status "$PROXY_SOL" sendTransaction "[\"$CLEAN\",{\"encoding\":\"base64\"}]")
[ "$status" = "200" ] && ok "forwarded (HTTP $status)" || warn "expected 200, got $status"

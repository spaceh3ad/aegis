#!/usr/bin/env bash
# CASE 2 — Spam (network-level congestion).
#   High-compute, zero-fee transactions are the classic spam class. Aegis refuses them at the
#   edge so the node never reserves the compute; paid traffic still flows.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

SPAM="${SPAM:-100}"

step "0. Load the spam rule (Wasm)"
set_rule zero-fee-filter
note "rule: cu_requested > 400000 AND priority_fee == 0  -> block"

step "1. A paid, modest transaction — allowed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 200000 1000)")
expect_allow "$code" "$rule" "200k CU with a priority fee"
note "watch terminal 2: 'allowed  cu=200000 fee=1000'"

step "2. $SPAM zero-fee, 500k-CU spam transactions"
blocked=0; forwarded=0
for _ in $(seq 1 "$SPAM"); do
  read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)")
  if [ "$rule" != "-" ]; then blocked=$((blocked+1)); else forwarded=$((forwarded+1)); fi
done
[ "$forwarded" -eq 0 ] && ok "$blocked/$SPAM blocked, none forwarded" || warn "$blocked blocked, $forwarded leaked"
note "compute the validator never reserved: $((blocked*500000)) CU"

step "3. Spam is off, a paid tx still passes (no false positives)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 5000)")
expect_allow "$code" "$rule" "same CU, but paying a fee"

step "Done — the volume is visible in terminal 2, one BLOCKED line per spam tx."

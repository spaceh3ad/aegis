#!/usr/bin/env bash
# CASE 2 — spam (high-CU, zero-fee burst).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
SPAM="${SPAM:-100}"

say "load spam rule"
set_rule zero-fee-filter

say "paid tx (200k CU, fee) — allowed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 200000 1000)" "200k CU, fee 1000"); report_allow "$rule"

say "$SPAM zero-fee 500k-CU txs — blocked at the edge"
blocked=0
for i in $(seq 1 "$SPAM"); do
  lbl=""; [ "$i" = 1 ] && lbl="500k CU, fee 0  (×$SPAM)"
  read -r _c rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 0)" "$lbl")
  [ "$rule" != "-" ] && blocked=$((blocked+1))
done
ok "$blocked/$SPAM blocked"
note "compute the validator never reserved: $((blocked*500000)) CU"

say "same CU, but paying a fee — allowed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" spam 500000 5000)" "500k CU, fee 5000"); report_allow "$rule"

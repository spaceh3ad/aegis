#!/usr/bin/env bash
# CASE 1 — Blacklist.
#   a) a sender/recipient on the list is blocked, before the validator sees it;
#   b) ADVANCED: a sanctioned recipient is passed ONLY through an Address Lookup Table,
#      so it is absent from the static keys — Aegis still resolves and blocks it.
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

USER=$(art user.pub); SAN=$(art sanctioned.pub); LUT=$(art lut.txt); PAY=$(art payprog.txt)
[ -n "$SAN" ] || { warn "run demo/cases/setup.sh first"; exit 1; }

step "0. Baseline — a normal transfer is allowed"
reset_blocklist
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$(python3 -c "import base58;print(base58.b58decode('$SAN').hex())")")")
expect_allow "$code" "$rule" "unlisted recipient"

step "1. Add the recipient to the operator list, hot-reload (SIGHUP)"
use_blocklist "$SAN"
note "blocklist.txt + 1 line; sent SIGHUP; no restart"
note "watch terminal 2: 'blocklist reloaded'"

step "2. Same transaction, now blocked at the edge"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$(python3 -c "import base58;print(base58.b58decode('$SAN').hex())")")")
expect_block "$code" "$rule" "recipient now on the list"

if [ -n "$LUT" ] && [ -n "$PAY" ]; then
  step "3. ADVANCED — sanction delivered via Address Lookup Table"
  say "the sanctioned address is loaded through the LUT, not in the static message"
  read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" lut "$PAY" "$LUT")")
  expect_block "$code" "$rule" "LUT-loaded sanctioned account (static keys never contain it)"
  note "open the LUT: solana address-lookup-table get $LUT --url $VALIDATOR"
fi

step "Done — check terminal 2: every decision is logged with its rule."

#!/usr/bin/env bash
# CASE 1 — blacklist (basic + LUT-delivered sanction).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
SAN=$(art sanctioned.pub); LUT=$(art lut.txt); PAY=$(art payprog.txt)
SAN_HEX=$(python3 -c "import base58;print(base58.b58decode('$SAN').hex())")

say "clean recipient (not listed)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$SAN_HEX")"); report_allow "$rule"

say "add recipient to the list + hot-reload"
use_blocklist "$SAN"; note "SIGHUP sent"

say "same recipient, now listed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$SAN_HEX")"); report_block "$rule"

say "sanction delivered only via the lookup table (not a static key)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" lut "$PAY" "$LUT")"); report_block "$rule"

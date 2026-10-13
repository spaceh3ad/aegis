#!/usr/bin/env bash
# CASE 1 — blacklist (basic + LUT-delivered sanction).
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"; require_aegis
SAN=$(art sanctioned.pub); LUT=$(art lut.txt); PAY=$(art payprog.txt)
SAN_HEX=$(python3 "$B58" "$SAN")

# Self-contained: start from the repo baseline so a re-run doesn't inherit the key this case
# appended on a previous run (which would make the "clean recipient" step falsely blocked).
reset_blocklist

say "clean recipient (not listed)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$SAN_HEX")"); report_allow "$rule"

say "add recipient to the list + hot-reload"
use_blocklist "$SAN"; note "SIGHUP sent"

say "same recipient, now listed"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" simple "$SAN_HEX")"); report_block "$rule"

say "sanction delivered only via the lookup table (not a static key)"
read -r code rule < <(send_sol "$(python3 "$CASES/txbuild.py" lut "$PAY" "$LUT")"); report_block "$rule"

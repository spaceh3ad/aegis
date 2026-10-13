#!/usr/bin/env bash
# One-time setup for the demo: generate keys, fund them, build+deploy the fixtures, create
# the Address Lookup Table. Run once after `bash demo/proxy.sh` is up.
#
#   bash demo/cases/setup.sh
#
# Artifacts land in demo/.run/ (gitignored): user.json, sanctioned.json, lut.txt,
# erc20.txt, payprog.txt, sbf/payments_program.so.
set -uo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"
require_stack

step "Funded accounts"
[ -f "$RUN/user.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/user.json" >/dev/null 2>&1
[ -f "$RUN/sanctioned.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/sanctioned.json" >/dev/null 2>&1
USER_PK=$(solana-keygen pubkey "$RUN/user.json")
SAN_PK=$(solana-keygen pubkey "$RUN/sanctioned.json")
echo "$USER_PK" > "$RUN/user.pub"; echo "$SAN_PK" > "$RUN/sanctioned.pub"
solana airdrop 5 "$USER_PK" --url "$VALIDATOR" >/dev/null 2>&1
solana airdrop 5 "$SAN_PK"  --url "$VALIDATOR" >/dev/null 2>&1
ok "user       $USER_PK"
ok "sanctioned $SAN_PK"

step "Address Lookup Table"
if [ -f "$RUN/lut.txt" ]; then
  LUT=$(cat "$RUN/lut.txt"); note "reusing LUT $LUT"
else
  LUT=$(solana address-lookup-table create --authority "$RUN/user.json" --url "$VALIDATOR" \
        | awk '/Lookup Table Address/{print $NF}')
  echo "$LUT" > "$RUN/lut.txt"
fi
# The sanctioned recipient lives *inside the table* — this is the whole point of the LUT case.
solana address-lookup-table extend "$LUT" --authority "$RUN/user.json" \
  --url "$VALIDATOR" --addresses "$SAN_PK" >/dev/null 2>&1 || true
ADDRS=$(solana address-lookup-table get "$LUT" --url "$VALIDATOR" --output json 2>/dev/null \
  | python3 -c "import sys,json;print(','.join(json.load(sys.stdin).get('addresses',[])))" 2>/dev/null)
ok "LUT $LUT  [${ADDRS:-none}]"

step "Solana program (payments-program)"
if [ -f "$RUN/payprog.txt" ]; then
  note "reusing $(cat "$RUN/payprog.txt")"
else
  (cd "$FIX/solana/payments-program" && cargo-build-sbf --sbf-out-dir "$RUN/sbf" >/dev/null 2>&1) \
    || { warn "cargo-build-sbf failed"; exit 1; }
  [ -f "$RUN/payprog-keypair.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/payprog-keypair.json" >/dev/null 2>&1
  solana program deploy --program-id "$RUN/payprog-keypair.json" "$RUN/sbf/payments_program.so" \
    --url "$VALIDATOR" >/dev/null 2>&1 || { warn "program deploy failed"; exit 1; }
  solana-keygen pubkey "$RUN/payprog-keypair.json" > "$RUN/payprog.txt"
fi
ok "program $(cat "$RUN/payprog.txt")"

step "ERC-20 (DemoToken on anvil)"
if [ -f "$RUN/erc20.txt" ]; then
  note "reusing $(cat "$RUN/erc20.txt")"
else
  OUT=$(cd "$FIX/erc20" && forge create \
        --rpc-url "$ANVIL" --private-key "$EVM_KEY" --broadcast \
        src/DemoToken.sol:DemoToken --constructor-args 1000000000000000000000000 2>&1)
  ADDR=$(echo "$OUT" | awk '/Deployed to/{print $NF}')
  [ -n "$ADDR" ] || { warn "forge create failed"; echo "$OUT"; exit 1; }
  echo "$ADDR" > "$RUN/erc20.txt"
fi
ok "token $(cat "$RUN/erc20.txt")"

echo
ok "setup complete — run a case, e.g. bash demo/cases/blacklist.sh"

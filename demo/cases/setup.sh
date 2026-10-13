#!/usr/bin/env bash
# One-time setup. Run this AFTER your host chains are up.
#
#   # terminal A
#   solana-test-validator --reset --quiet --rpc-port 8900
#   # terminal B
#   anvil --port 8545 --chain-id 31337
#   # terminal C
#   bash demo/cases/setup.sh
#
# It: seeds the policy files demo/.run/ (which Aegis mounts), starts Aegis in Docker,
# then builds the demo fixtures (funded keys, an Address Lookup Table, a Solana program,
# an ERC-20 on anvil). Idempotent — safe to re-run.
set -uo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

say "== preconditions =="
curl -sf -X POST "$VALIDATOR" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' >/dev/null 2>&1 \
  && ok "validator  $VALIDATOR" || { warn "no validator at $VALIDATOR"; warn "  solana-test-validator --reset --quiet --rpc-port 8900"; exit 1; }
curl -sf -X POST "$ANVIL" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId"}' >/dev/null 2>&1 \
  && ok "anvil      $ANVIL" || { warn "no anvil at $ANVIL"; warn "  anvil --port 8545 --chain-id 31337"; exit 1; }

say "== policy files (mounted by Aegis) =="
mkdir -p "$RUN"
# Always reseed from the repo: case runs append test addresses to demo/.run/blocklist.txt,
# so a re-run must start clean or the "clean recipient" step fails.
cp "$ROOT/blocklist.txt" "$RUN/blocklist.txt"
[ -f "$RUN/rule.wasm" ] || cp "$ROOT/plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm" "$RUN/rule.wasm"
ok "demo/.run/{blocklist.txt,rule.wasm} seeded clean"

say "== Aegis (docker) =="
if curl -sf "$PROXY_SOL/metrics" >/dev/null 2>&1; then
  ok "aegis already up at $PROXY_SOL"
else
  (cd "$ROOT" && docker compose up -d --build) >/dev/null 2>&1 || { warn "docker compose up failed"; exit 1; }
  for _ in $(seq 1 60); do curl -sf "$PROXY_SOL/metrics" >/dev/null 2>&1 && break; sleep 0.5; done
  curl -sf "$PROXY_SOL/metrics" >/dev/null 2>&1 && ok "aegis(sol) $PROXY_SOL   aegis(evm) $PROXY_EVM" \
    || { warn "aegis did not come up; docker compose logs"; exit 1; }
fi
note "watch decisions:  docker compose logs -f aegis-sol aegis-evm"

say "== fixtures =="
[ -f "$RUN/user.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/user.json" >/dev/null 2>&1
[ -f "$RUN/sanctioned.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/sanctioned.json" >/dev/null 2>&1
USER_PK=$(solana-keygen pubkey "$RUN/user.json"); SAN_PK=$(solana-keygen pubkey "$RUN/sanctioned.json")
echo "$USER_PK" > "$RUN/user.pub"; echo "$SAN_PK" > "$RUN/sanctioned.pub"
solana airdrop 5 "$USER_PK" --url "$VALIDATOR" >/dev/null 2>&1
solana airdrop 5 "$SAN_PK"  --url "$VALIDATOR" >/dev/null 2>&1
ok "funded user $USER_PK  sanctioned $SAN_PK"

# A validator --reset wipes accounts, so verify artifacts still exist before reusing them.
sol_exists(){ # $1 = base58 pubkey -> 0 if the account exists on the validator
  local v
  v=$(curl -s -X POST "$VALIDATOR" -H 'content-type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccountInfo\",\"params\":[\"$1\",{\"encoding\":\"base64\"}]}" \
      | python3 -c "import sys,json;print(json.load(sys.stdin).get('result',{}).get('value') is not None)" 2>/dev/null)
  [ "$v" = "True" ]
}
evm_code_exists(){ # $1 = 0x address -> 0 if contract code exists on anvil
  local c
  c=$(curl -s -X POST "$ANVIL" -H 'content-type: application/json' \
      -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_getCode\",\"params\":[\"$1\",\"latest\"]}" \
      | python3 -c "import sys,json;print(json.load(sys.stdin).get('result','0x'))" 2>/dev/null)
  [ -n "$c" ] && [ "$c" != "0x" ]
}

if [ -f "$RUN/lut.txt" ] && sol_exists "$(cat "$RUN/lut.txt")"; then
  LUT=$(cat "$RUN/lut.txt")
else
  LUT=$(solana address-lookup-table create --authority "$RUN/user.json" --url "$VALIDATOR" | awk '/Lookup Table Address/{print $NF}')
  echo "$LUT" > "$RUN/lut.txt"
fi
solana address-lookup-table extend "$LUT" --authority "$RUN/user.json" --url "$VALIDATOR" --addresses "$SAN_PK" >/dev/null 2>&1 || true
ok "lookup table $LUT (contains the sanctioned address)"

if [ -f "$RUN/payprog.txt" ] && sol_exists "$(cat "$RUN/payprog.txt")"; then
  ok "program $(cat "$RUN/payprog.txt")"
else
  (cd "$FIX/solana/payments-program" && cargo-build-sbf --sbf-out-dir "$RUN/sbf" >/dev/null 2>&1) || { warn "cargo-build-sbf failed"; exit 1; }
  [ -f "$RUN/payprog-keypair.json" ] || solana-keygen new --no-bip39-passphrase -s -o "$RUN/payprog-keypair.json" >/dev/null 2>&1
  solana program deploy --program-id "$RUN/payprog-keypair.json" "$RUN/sbf/payments_program.so" --url "$VALIDATOR" >/dev/null 2>&1 || { warn "program deploy failed"; exit 1; }
  solana-keygen pubkey "$RUN/payprog-keypair.json" > "$RUN/payprog.txt"
  ok "program $(cat "$RUN/payprog.txt")"
fi

if [ -f "$RUN/erc20.txt" ] && evm_code_exists "$(cat "$RUN/erc20.txt")"; then
  ok "erc20 $(cat "$RUN/erc20.txt")"
else
  OUT=$(cd "$FIX/erc20" && forge create --rpc-url "$ANVIL" --private-key "$EVM_KEY" --broadcast \
        src/DemoToken.sol:DemoToken --constructor-args 1000000000000000000000000 2>&1)
  ADDR=$(echo "$OUT" | awk '/Deployed to/{print $NF}')
  [ -n "$ADDR" ] || { warn "forge create failed"; echo "$OUT"; exit 1; }
  echo "$ADDR" > "$RUN/erc20.txt"
  ok "erc20 $ADDR"
fi

echo
ok "ready. run a case, e.g.  bash demo/cases/blacklist.sh"

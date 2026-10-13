#!/usr/bin/env bash
# Terminal 2: run the stack and watch Aegis.
#
#   bash demo/proxy.sh
#
# Boots:
#   solana-test-validator  :8900
#   anvil                  :8545
#   aegis (role=sol)       :8899 -> validator
#   aegis (role=evm)       :8898 -> anvil
#
# Both Aegis instances are started with LOG_FORMAT=pretty and watch two fixed files that the
# case scripts edit to change policy live (no restart):
#   demo/.run/blocklist.txt   (address list)
#   demo/.run/rule.wasm       (Wasm rule plugin)
#
# Ctrl-C stops everything.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUN="$ROOT/demo/.run"
BIN="$ROOT/target/release/aegis-proxy"
mkdir -p "$RUN"
cp "$ROOT/blocklist.txt" "$RUN/blocklist.txt"
cp "$ROOT/plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm" "$RUN/rule.wasm" 2>/dev/null || true

B='\033[1m'; C='\033[0;36m'; Y='\033[1;33m'; R='\033[0;31m'; X='\033[0m'
info(){ echo -e "  ${Y}•${X} $*"; }
head2(){ echo -e "\n${B}${C}━━━ $* ━━━${X}"; }

PIDS=()
cleanup(){
  echo -e "\n${R}stopping stack...${X}"
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  pkill -f solana-test-validator 2>/dev/null || true
  pkill -f 'aegis-proxy sol' 2>/dev/null || true
  pkill -f 'aegis-proxy evm' 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

[ -x "$BIN" ] || { info "building release binary..."; cargo build --release -p aegis-proxy >/dev/null 2>&1; }
for p in zero-fee-filter approve-guard program-guard silent-drop; do
  w="$ROOT/plugins/$p/target/wasm32-wasip1/release/${p//-/_}.wasm"
  [ -f "$w" ] || (cd "$ROOT/plugins/$p" && cargo build --release --target wasm32-wasip1 >/dev/null 2>&1)
done

head2 "STACK"
info "validator :8900   anvil :8545   aegis(sol) :8899   aegis(evm) :8898"
info "terminal 1: run a case, e.g. bash demo/cases/blacklist.sh"
echo

# ── validator ───────────────────────────────────────────────────────────────
solana-test-validator --reset --quiet --rpc-port 8900 >"$RUN/validator.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 80); do curl -sf -X POST http://127.0.0.1:8900 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' >/dev/null 2>&1 && break; sleep 0.5; done
info "validator up"

# ── anvil ───────────────────────────────────────────────────────────────────
anvil --silent --port 8545 --chain-id 31337 >"$RUN/anvil.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 40); do curl -sf -X POST http://127.0.0.1:8545 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId"}' >/dev/null 2>&1 && break; sleep 0.25; done
info "anvil up"

# ── aegis x2 ────────────────────────────────────────────────────────────────
DOWNSTREAM_URL="http://127.0.0.1:8900" LISTEN_ADDR="127.0.0.1:8899" \
  BLOCKLIST_PATH="$RUN/blocklist.txt" WASM_PLUGIN_PATH="$RUN/rule.wasm" LOG_FORMAT=pretty \
  "$BIN" sol >>"$RUN/aegis.log" 2>&1 &
PIDS+=($!)
DOWNSTREAM_URL="http://127.0.0.1:8545" LISTEN_ADDR="127.0.0.1:8898" \
  BLOCKLIST_PATH="$RUN/blocklist.txt" WASM_PLUGIN_PATH="$RUN/rule.wasm" LOG_FORMAT=pretty \
  "$BIN" evm >>"$RUN/aegis.log" 2>&1 &
PIDS+=($!)
for _ in $(seq 1 40); do curl -sf http://127.0.0.1:8899/metrics >/dev/null 2>&1 && break; sleep 0.25; done
info "aegis(sol) up, aegis(evm) up"
echo

# ── tail the proxy log (the thing you watch) ────────────────────────────────
head2 "AEGIS LIVE — screening (Ctrl-C to stop)"
tail -n 0 -F "$RUN/aegis.log"

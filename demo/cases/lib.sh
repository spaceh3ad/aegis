#!/usr/bin/env bash
# Shared helpers for the Aegis demo cases. Source this; do not run it.
#
# Topology you set up:
#   host:  solana-test-validator :8900      anvil :8545
#   docker: aegis-sol :8899 -> validator    aegis-evm :8898 -> anvil
#
# The case scripts only SEND transactions to the Aegis ports. Watch the decisions in the
# Docker log:  docker compose logs -f aegis-sol aegis-evm
#
# Policy lives in demo/.run/ (mounted into both containers at /app/policy). Changing policy =
# overwrite those files, then `docker compose kill -s HUP` — the running containers reload
# without restarting.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CASES="$ROOT/demo/cases"
FIX="$ROOT/demo/fixtures"
RUN="$ROOT/demo/.run"
B58="$ROOT/demo/b58.py"   # vendored pure-Python base58 (no third-party deps)

# Aegis (Docker) endpoints the scripts talk to.
PROXY_SOL="${AEGIS_SOL_PROXY:-http://127.0.0.1:8899}"
PROXY_EVM="${AEGIS_EVM_PROXY:-http://127.0.0.1:8898}"
# Host chains (for setup.sh only — the scripts themselves go through Aegis).
VALIDATOR="${AEGIS_VALIDATOR_URL:-http://127.0.0.1:8900}"
ANVIL="${AEGIS_ANVIL_URL:-http://127.0.0.1:8545}"

EVM_FROM="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"   # anvil account #0
EVM_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

# txbuild.py vendors base58 (no third-party deps), but evmtx.py signs with eth_account, which
# not every python3 has. Pick the first interpreter that can import it. Override with AEGIS_PYTHON.
PY="${AEGIS_PYTHON:-}"
if [ -z "$PY" ]; then
  for cand in python3 /opt/anaconda3/bin/python3 /opt/homebrew/bin/python3; do
    if command -v "$cand" >/dev/null 2>&1 && "$cand" -c 'import eth_account' >/dev/null 2>&1; then PY="$cand"; break; fi
  done
  [ -z "$PY" ] && PY=python3
fi

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; X='\033[0m'
say(){  echo -e "${B}$*${X}"; }
ok(){   echo -e "  ${G}✓${X} $*"; }
warn(){ echo -e "  ${R}!${X} $*"; }
note(){ echo -e "  ${Y}·${X} $*"; }
# Request trace, to stderr so it survives the `read < <(send_*)` command substitutions.
trace(){ echo -e "  ${C}→${X} $*" >&2; }

# ── policy control: overwrite files, then SIGHUP the containers ──────────────
# Requires the stack to be up (`docker compose up -d`). Falls back to pkill for a local run.
sig_proxy(){
  if docker compose -f "$ROOT/docker-compose.yml" ps --status running >/dev/null 2>&1 \
     && [ -n "$(docker compose -f "$ROOT/docker-compose.yml" ps -q aegis-sol 2>/dev/null)" ]; then
    docker compose -f "$ROOT/docker-compose.yml" kill -s HUP aegis-sol aegis-evm >/dev/null 2>&1
  else
    pkill -HUP -f 'aegis-proxy sol' 2>/dev/null; pkill -HUP -f 'aegis-proxy evm' 2>/dev/null
  fi
  sleep 0.5
}

set_rule(){ # $1 = plugin name; empty = noop
  [ -z "$1" ] && return 0
  cp "$ROOT/plugins/$1/target/wasm32-wasip1/release/${1//-/_}.wasm" "$RUN/rule.wasm"
  sig_proxy
}
use_blocklist(){ for a in "$@"; do grep -qxF "$a" "$RUN/blocklist.txt" 2>/dev/null || echo "$a" >> "$RUN/blocklist.txt"; done; sig_proxy; }
reset_blocklist(){ cp "$ROOT/blocklist.txt" "$RUN/blocklist.txt"; sig_proxy; }

require_aegis(){
  curl -sf "$PROXY_SOL/metrics" >/dev/null 2>&1 || { warn "Aegis not reachable at $PROXY_SOL"; warn "start it:  bash demo/cases/setup.sh (after starting the host chains)"; exit 1; }
}

# ── JSON-RPC ────────────────────────────────────────────────────────────────
rpc(){ curl -s -m 10 -X POST "$1" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }
rpc_status(){ curl -s -m 10 -o /dev/null -w '%{http_code}' -X POST "$1" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }

# Send a tx; print "<code> <rule_id|->". rule_id present == Aegis blocked it.
# Optional $2 is a human label, traced to stderr (survives the caller's command substitution).
send_sol(){ [ -n "${2:-}" ] && trace "sendTransaction  $2"
  rpc "$PROXY_SOL" sendTransaction "[\"$1\",{\"encoding\":\"base64\"}]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','ok'), (e.get('data') or {}).get('rule_id') or '-')" 2>/dev/null; }

send_evm(){ [ -n "${2:-}" ] && trace "eth_sendRawTransaction  $2"
  rpc "$PROXY_EVM" eth_sendRawTransaction "[\"$1\"]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','ok'), (e.get('data') or {}).get('rule_id') or '-')" 2>/dev/null; }

# Thin reporters: one line each.
report_block(){ [ "$1" != "-" ] && ok "blocked by $1" || warn "expected a block, but Aegis returned no rule_id (request reached the node)"; }
report_allow(){ [ "$1" = "-" ] && ok "allowed" || warn "unexpectedly blocked by $1"; }
report_drop(){  [ "$1" = "204" ] && ok "dropped (HTTP 204)" || warn "expected 204, got $1"; }

art(){ cat "$RUN/$1" 2>/dev/null; }
evm_nonce(){ rpc "$ANVIL" eth_getTransactionCount "[\"$EVM_FROM\",\"latest\"]" | python3 -c "import sys,json;print(int(json.load(sys.stdin).get('result','0x0'),16))"; }

#!/usr/bin/env bash
# Shared helpers for the Aegis demo cases. Source this; do not run it.
#
# Topology (started by demo/proxy.sh):
#   validator :8900            anvil :8545
#   aegis (sol) :8899 -> validator      aegis (evm) :8898 -> anvil
#
# Aegis fronts ONE upstream, so one instance per chain — the same binary twice.
# Terminal 1: run a case script here.  Terminal 2: `bash demo/proxy.sh` (pretty logs).
#
# The proxies are started with BLOCKLIST_PATH=$RUN/blocklist.txt and
# WASM_PLUGIN_PATH=$RUN/rule.wasm, and reload BOTH on SIGHUP from those fixed paths.
# So a case "changes policy" by overwriting those files and signalling — that is the
# live-reload story, not a restart.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CASES="$ROOT/demo/cases"
FIX="$ROOT/demo/fixtures"
RUN="$ROOT/demo/.run"
mkdir -p "$RUN"

PROXY_SOL="${AEGIS_SOL_PROXY:-http://127.0.0.1:8899}"
PROXY_EVM="${AEGIS_EVM_PROXY:-http://127.0.0.1:8898}"
VALIDATOR="${AEGIS_VALIDATOR_URL:-http://127.0.0.1:8900}"
ANVIL="${AEGIS_ANVIL_URL:-http://127.0.0.1:8545}"

EVM_FROM="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"   # anvil account #0
EVM_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; X='\033[0m'
step(){ echo -e "\n${B}${C}▸ $*${X}"; }
ok(){   echo -e "  ${G}✓${X} $*"; }
warn(){ echo -e "  ${R}!${X} $*"; }
note(){ echo -e "  ${Y}•${X} $*"; }
say(){  echo -e "  ${B}$*${X}"; }
need(){ command -v "$1" >/dev/null 2>&1 || { warn "missing tool: $1"; exit 1; }; }

# ── live policy control (overwrite the watched files, then SIGHUP) ──────────
sig_proxy(){ pkill -HUP -f 'aegis-proxy sol' 2>/dev/null; pkill -HUP -f 'aegis-proxy evm' 2>/dev/null; sleep 0.4; }

set_rule(){ # $1 = plugin name (zero-fee-filter|approve-guard|program-guard|silent-drop); empty = noop
  [ -z "$1" ] && return 0
  cp "$ROOT/plugins/$1/target/wasm32-wasip1/release/${1//-/_}.wasm" "$RUN/rule.wasm"
  sig_proxy
}
use_blocklist(){ for a in "$@"; do grep -qxF "$a" "$RUN/blocklist.txt" 2>/dev/null || echo "$a" >> "$RUN/blocklist.txt"; done; sig_proxy; }
reset_blocklist(){ cp "$ROOT/blocklist.txt" "$RUN/blocklist.txt"; sig_proxy; }

wait_proxy(){ for _ in $(seq 1 40); do curl -sf "$1/metrics" >/dev/null 2>&1 && return 0; sleep 0.25; done; warn "proxy not reachable at $1"; return 1; }
require_stack(){ wait_proxy "$PROXY_SOL" || { warn "start terminal 2 first: bash demo/proxy.sh"; exit 1; }; }

# ── JSON-RPC ────────────────────────────────────────────────────────────────
rpc(){ curl -s -m 10 -X POST "$1" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }
rpc_status(){ curl -s -m 10 -o /dev/null -w '%{http_code}' -X POST "$1" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }

send_sol(){ rpc "$PROXY_SOL" sendTransaction "[\"$1\",{\"encoding\":\"base64\"}]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id') or '-')" 2>/dev/null; }

send_evm(){ rpc "$PROXY_EVM" eth_sendRawTransaction "[\"$1\"]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id') or '-', str(d.get('result',''))[:20] or '-')" 2>/dev/null; }

# ── verdict helpers (Aegis blocks carry error.data.rule_id; node errors do not) ─
# An empty rule_id prints as '-', so field positions never shift.
_norm(){ [ "$1" = "-" ] && echo "" || echo "$1"; }
expect_block(){ local r; r=$(_norm "$2"); if [ -n "$r" ]; then ok "$3 — blocked at proxy (rule=$r)"; else warn "$3 — expected Aegis block, got code=$1"; return 1; fi; }
expect_allow(){ local r; r=$(_norm "$2"); if [ -z "$r" ]; then ok "$3 — passed inspection"; else warn "$3 — unexpectedly blocked by $r"; return 1; fi; }
expect_drop(){ if [ "$1" = "204" ]; then ok "$2 — silently dropped (HTTP 204, empty body)"; else warn "$2 — expected 204, got HTTP $1"; return 1; fi; }

art(){ cat "$RUN/$1" 2>/dev/null; }
evm_nonce(){ rpc "$ANVIL" eth_getTransactionCount "[\"$EVM_FROM\",\"latest\"]" | python3 -c "import sys,json;print(int(json.load(sys.stdin).get('result','0x0'),16))"; }

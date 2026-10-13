#!/usr/bin/env bash
# Shared helpers for the Aegis demo cases. Source this; do not run it.
#
# Topology (see demo/proxy.sh):
#   validator  :8900          anvil :8545
#   aegis-sol  :8899 -> validator      aegis-evm :8898 -> anvil
#
# Aegis fronts ONE upstream, so there is one instance per chain — the same binary twice.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CASES="$ROOT/demo/cases"
FIX="$ROOT/demo/fixtures"
RUN="$ROOT/demo/.run"
mkdir -p "$RUN"

PROXY_SOL="${AEGIS_SOL_PROXY:-http://127.0.0.1:8899}"
PROXY_EVM="${AEGIS_EVM_PROXY:-http://127.0.0.1:8898}"
VALIDATOR="${AEGIS_VALIDATOR_URL:-http://127.0.0.1:8900}"
ANVIL="${AEGIS_ANVIL_URL:-http://127.0.0.1:8545}"

# anvil account #0 (funded, chain-id 31337) — the demo's EVM signer.
EVM_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
EVM_FROM="0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; X='\033[0m'
step(){ echo -e "\n${B}${C}▸ $*${X}"; }
ok(){   echo -e "  ${G}✓${X} $*"; }
warn(){ echo -e "  ${R}!${X} $*"; }
note(){ echo -e "  ${Y}•${X} $*"; }
say(){  echo -e "  ${B}$*${X}"; }
need(){ command -v "$1" >/dev/null 2>&1 || { warn "missing tool: $1"; exit 1; }; }

# ── rule + blocklist control (drives the running proxy via SIGHUP) ───────────
set_rule(){ # $1 = plugin name (zero-fee-filter|approve-guard|program-guard|silent-drop)
  cp "$ROOT/plugins/$1/target/wasm32-wasip1/release/${1//-/_}.wasm" "$RUN/rule.wasm"
  sig_proxy
}
use_blocklist(){ # $1.. = addresses to ADD to the live list
  for a in "$@"; do grep -qxF "$a" "$RUN/blocklist.txt" 2>/dev/null || echo "$a" >> "$RUN/blocklist.txt"; done
  sig_proxy
}
reset_blocklist(){ cp "$ROOT/blocklist.txt" "$RUN/blocklist.txt"; sig_proxy; }
sig_proxy(){ pkill -HUP -f 'aegis-proxy.*aegis-sol' 2>/dev/null; pkill -HUP -f 'aegis-proxy.*aegis-evm' 2>/dev/null; sleep 0.4; }

wait_proxy(){ # $1 = url
  for _ in $(seq 1 40); do curl -sf "$1/metrics" >/dev/null 2>&1 && return 0; sleep 0.25; done
  warn "proxy not reachable at $1"; return 1; }
require_stack(){ wait_proxy "$PROXY_SOL" || { warn "start demo/proxy.sh first"; exit 1; }; }

# ── JSON-RPC ────────────────────────────────────────────────────────────────
rpc(){ curl -s -m 10 -X POST "$1" -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }
rpc_status(){ curl -s -m 10 -o /dev/null -w '%{http_code}' -X POST "$1" -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$2\",\"params\":$3}"; }

send_sol(){ rpc "$PROXY_SOL" sendTransaction "[\"$1\",{\"encoding\":\"base64\"}]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id',''))" 2>/dev/null; }

send_evm(){ rpc "$PROXY_EVM" eth_sendRawTransaction "[\"$1\"]" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id',''), str(d.get('result',''))[:20])" 2>/dev/null; }

# ── verdict assertions ──────────────────────────────────────────────────────
# Aegis blocks carry error.data.rule_id; downstream node errors do not.
expect_block(){ if [ -n "$2" ]; then ok "$3 — blocked at proxy (rule=$2)"; else warn "$3 — expected an Aegis block, got code=$1"; return 1; fi; }
expect_allow(){ if [ -z "$2" ]; then ok "$3 — passed inspection"; else warn "$3 — unexpectedly blocked by $2"; return 1; fi; }
expect_drop(){ if [ "$1" = "204" ]; then ok "$2 — dropped silently (HTTP 204, no body)"; else warn "$2 — expected 204, got HTTP $1"; return 1; fi; }

# ── artifacts from setup.sh ─────────────────────────────────────────────────
art(){ cat "$RUN/$1" 2>/dev/null; }

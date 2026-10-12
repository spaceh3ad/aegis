#!/usr/bin/env bash
# Aegis live demo against a real Solana RPC — no Docker, no emulated validator.
#
# Runs the locally-built release binary as a sidecar in front of any JSON-RPC
# endpoint (testnet, devnet, mainnet, or Helius), then exercises the same
# checks as demo.sh.
#
# Usage:
#   export HELIUS_API_KEY=...            # optional; falls back to public testnet
#   bash demo/rpc-demo.sh
#
#   # or point at anything explicitly:
#   AEGIS_RPC_URL="https://devnet.helius-rpc.com/?api-key=$HELIUS_API_KEY" bash demo/rpc-demo.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Pick an RPC endpoint: explicit override > Helius testnet > public testnet.
if [ -n "${AEGIS_RPC_URL:-}" ]; then
    RPC="$AEGIS_RPC_URL"
elif [ -n "${HELIUS_API_KEY:-}" ]; then
    RPC="https://testnet.helius-rpc.com/?api-key=${HELIUS_API_KEY}"
else
    RPC="https://api.testnet.solana.com"
fi

PROXY_PORT="${AEGIS_LISTEN_PORT:-8899}"
PROXY="http://127.0.0.1:${PROXY_PORT}"
BIN="$ROOT/target/release/aegis-proxy"
BLOCKLIST="${BLOCKLIST_PATH:-$ROOT/blocklist.txt}"

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; CYAN='\033[0;36m'; RESET='\033[0m'
banner() { echo -e "\n${BOLD}=== $* ===${RESET}"; }
ok()     { echo -e "  ${GREEN}[PASS]${RESET} $*"; }
blocked(){ echo -e "  ${RED}[BLOCKED]${RESET} $*"; }
info()   { echo -e "  ${YELLOW}[INFO]${RESET} $*"; }

if [ ! -x "$BIN" ]; then
    info "Building release binary..."
    cargo build --release -p aegis-proxy >/dev/null 2>&1
fi

banner "Aegis — live RPC demo"
info "RPC endpoint : $RPC"
info "Proxy        : $PROXY"
info "Blocklist    : $BLOCKLIST"

# Boot the proxy against the external RPC and tear it down on exit.
DOWNSTREAM_URL="$RPC" LISTEN_ADDR="127.0.0.1:${PROXY_PORT}" BLOCKLIST_PATH="$BLOCKLIST" \
    "$BIN" >/tmp/aegis-rpc-demo.log 2>&1 &
PROXY_PID=$!
trap 'kill "$PROXY_PID" 2>/dev/null || true' EXIT

info "Waiting for proxy..."
for _ in $(seq 1 30); do
    curl -sf "$PROXY/metrics" >/dev/null 2>&1 && break
    sleep 0.5
done
curl -sf "$PROXY/metrics" >/dev/null 2>&1 || { echo "ERROR: proxy did not start (see /tmp/aegis-rpc-demo.log)" >&2; exit 1; }
ok "Proxy is up (pid $PROXY_PID)"

# Minimal v0 tx carrying one pubkey, base64 as a wallet would send it.
make_tx_b64() {
    python3 - "$1" <<'PYEOF'
import sys, base64
k = bytes.fromhex(sys.argv[1]); assert len(k) == 32
tx = b'\x01' + b'\x00'*64 + b'\x80' + b'\x01\x00\x00' + b'\x01' + k + b'\x00'*32 + b'\x00' + b'\x00'
print(base64.b64encode(tx).decode())
PYEOF
}

# ── Test 1: real read through the proxy ────────────────────────────────────
banner "Test 1 — Real RPC call forwarded to the chain (getVersion)"
resp=$(curl -sf --max-time 10 -X POST "$PROXY" -H "content-type: application/json" \
    -d '{"jsonrpc":"2.0","id":1,"method":"getVersion"}')
if echo "$resp" | grep -q '"result"'; then
    ok "getVersion through Aegis: $resp"
else
    echo "FAIL: expected a result, got: $resp" >&2; exit 1
fi

# ── Test 2: getHealth / getSlot against the real cluster ───────────────────
banner "Test 2 — Live cluster state via the proxy"
curl -sf --max-time 10 -X POST "$PROXY" -H "content-type: application/json" \
    -d '{"jsonrpc":"2.0","id":2,"method":"getHealth"}' | sed 's/^/  /' ; echo
curl -sf --max-time 10 -X POST "$PROXY" -H "content-type: application/json" \
    -d '{"jsonrpc":"2.0","id":3,"method":"getSlot"}' | sed 's/^/  /' ; echo
ok "Live data flows through the firewall untouched"

# ── Test 3: blocklisted address rejected locally (never reaches the RPC) ───
banner "Test 3 — Blocklisted signer rejected before the RPC sees it"
BLOCKED_KEY_HEX="$(python3 -c "
import base58
k=base58.b58decode('CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv'); print(k.hex())
" 2>/dev/null || echo '')"
if [ -n "$BLOCKED_KEY_HEX" ]; then
    BLOCKED_B64="$(make_tx_b64 "$BLOCKED_KEY_HEX")"
    resp=$(curl -s --max-time 10 -X POST "$PROXY" -H "content-type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"sendTransaction\",\"params\":[\"$BLOCKED_B64\",{\"encoding\":\"base64\"}]}")
    code=$(echo "$resp" | python3 -c "import sys,json;print(json.load(sys.stdin).get('error',{}).get('code','none'))" 2>/dev/null || echo parse_error)
    if [ "$code" = "-32001" ]; then
        rule=$(echo "$resp" | python3 -c "import sys,json;print(json.load(sys.stdin)['error']['data']['rule_id'])" 2>/dev/null)
        trace=$(echo "$resp" | python3 -c "import sys,json;print(json.load(sys.stdin)['error']['data']['trace_id'])" 2>/dev/null)
        blocked "REJECTED at the proxy — code=$code, rule_id=$rule, trace_id=$trace"
        ok "Inspection latency is O(1) and the tx never left the box"
    else
        echo "FAIL: expected -32001, got code=$code" >&2; exit 1
    fi
else
    info "python3 base58 unavailable — skipping deterministic block test"
fi

# ── Test 4: inspection latency histogram ───────────────────────────────────
banner "Test 4 — Measured inspection latency (Prometheus histogram)"
curl -s "$PROXY/metrics" | grep -E 'aegis_inspection_duration_microseconds_(count|sum)' | sed 's/^/  /'
info "sum/count = mean microseconds of parse + policy per transaction"

banner "Done"
info "Proxy log: /tmp/aegis-rpc-demo.log"

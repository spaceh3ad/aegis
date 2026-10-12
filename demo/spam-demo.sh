#!/usr/bin/env bash
# Aegis spam demo — shows how much junk traffic never reaches the node.
#
# Boots a self-contained proxy with the zero-fee-filter Wasm rule in front of a real
# Solana RPC, then fires a burst of high-compute / zero-fee "spam" transactions and a
# handful of legitimate, fee-paying ones. Spam is answered at the proxy (never touches
# the network); legit transactions are forwarded.
#
# Usage:
#   bash demo/spam-demo.sh                     # public testnet
#   HELIUS_API_KEY=xxx bash demo/spam-demo.sh
#   AEGIS_RPC_URL="https://devnet.helius-rpc.com/?api-key=$KEY" bash demo/spam-demo.sh
#   SPAM=2000 bash demo/spam-demo.sh           # bigger burst
#   AEGIS_LISTEN_PORT=18899 bash demo/spam-demo.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ -n "${AEGIS_RPC_URL:-}" ]; then RPC="$AEGIS_RPC_URL"
elif [ -n "${HELIUS_API_KEY:-}" ]; then RPC="https://testnet.helius-rpc.com/?api-key=${HELIUS_API_KEY}"
else RPC="https://api.testnet.solana.com"; fi

PORT="${AEGIS_LISTEN_PORT:-18899}"
PROXY="http://127.0.0.1:${PORT}"
BIN="$ROOT/target/release/aegis-proxy"
WORK="$(mktemp -d)"
BL="$WORK/blocklist.txt"
ZFF="$ROOT/plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm"
RULE="$WORK/zero-fee-filter.wasm"

SPAM="${SPAM:-500}"
LEGIT="${LEGIT:-10}"

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; X='\033[0m'
banner(){ echo -e "\n${B}${C}━━━ $* ━━━${X}"; }
ok(){     echo -e "  ${G}✓${X} $*"; }
info(){   echo -e "  ${Y}•${X} $*"; }
row(){ printf "  %-8s %-24s %-24s %s\n" "$@"; }

cleanup(){ if [ -n "${PID:-}" ]; then kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; fi; rm -rf "$WORK"; }
trap cleanup EXIT
set +m

# ── Build ───────────────────────────────────────────────────────────────────
if [ ! -x "$BIN" ]; then info "Building release binary..."; cargo build --release -p aegis-proxy >/dev/null 2>&1; fi
if [ ! -f "$ZFF" ]; then (cd "$ROOT/plugins/zero-fee-filter" && cargo build --release --target wasm32-wasip1 >/dev/null 2>&1); fi
cp "$ROOT/blocklist.txt" "$BL"; cp "$ZFF" "$RULE"

# ── Boot ────────────────────────────────────────────────────────────────────
info "RPC endpoint : $RPC"
info "Proxy        : $PROXY"
DOWNSTREAM_URL="$RPC" LISTEN_ADDR="127.0.0.1:${PORT}" BLOCKLIST_PATH="$BL" WASM_PLUGIN_PATH="$RULE" \
    "$BIN" >"$WORK/aegis.log" 2>&1 &
PID=$!
for _ in $(seq 1 40); do curl -sf "$PROXY/metrics" >/dev/null 2>&1 && break; sleep 0.25; done
curl -sf "$PROXY/metrics" >/dev/null 2>&1 || { echo -e "${R}proxy failed to start:${X}"; tail -5 "$WORK/aegis.log"; exit 1; }
ok "Aegis up (pid $PID), rule: zero-fee-filter"

# ── tx builder: v0 tx requesting N CU, optionally paying a priority fee ─────
# $1 = cu_limit, $2 = cu_price (micro-lamports/CU; 0 = no SetComputeUnitPrice)
tx_b64(){ python3 - "$1" "$2" <<'PY'
import sys, base64, base58
cu, price = int(sys.argv[1]), int(sys.argv[2])
cb = base58.b58decode("ComputeBudget111111111111111111111111111111")
def u16(n):
    o=bytearray()
    while True:
        b=n&0x7f; n>>=7
        if n: o.append(b|0x80)
        else: o.append(b); break
    return bytes(o)
def instr(pidx, data): return bytes([pidx])+u16(0)+u16(len(data))+data
ixs=[instr(1, bytes([2])+cu.to_bytes(4,'little'))]           # SetComputeUnitLimit
if price>0: ixs.append(instr(1, bytes([3])+price.to_bytes(8,'little')))  # SetComputeUnitPrice
hdr=bytes([1,0,1]); accts=u16(2)+bytes(32)+cb; bh=bytes(32)
msg=hdr+accts+bh+u16(len(ixs))+b''.join(ixs)
print(base64.b64encode(u16(1)+bytes(64)+msg).decode())
PY
}

# sends one tx, prints the Aegis verdict: "blocked" | "reached"
# Authoritative discriminator is error.data.rule_id — JSON-RPC codes collide with the
# chain (Solana itself returns -32002 for a failed simulation), so a code alone is not
# proof that Aegis blocked it.
send_one(){ # $1 = base64 tx
  local resp verdict
  resp=$(curl -s -m 8 -X POST "$PROXY" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"sendTransaction\",\"params\":[\"$1\",{\"encoding\":\"base64\"}]}")
  verdict=$(printf '%s' "$resp" | python3 -c "import sys,json;print('blocked' if ((json.load(sys.stdin).get('error') or {}).get('data') or {}).get('rule_id') else 'reached')" 2>/dev/null || echo reached)
  echo "$verdict"
}

SPAM_TX="$(tx_b64 500000 0)"      # 500k CU, zero fee  -> spam
LEGIT_TX="$(tx_b64 200000 1000)"  # 200k CU, paid tip  -> legit

# ── Fire the spam burst ─────────────────────────────────────────────────────
banner "SPAM BURST — $SPAM txs, 500k CU each, zero priority fee"
info "each would reserve 500,000 compute units if the node accepted it"
spam_blocked=0; spam_reached=0
t0=$(python3 -c 'import time;print(time.time())')
for _ in $(seq 1 "$SPAM"); do
  [ "$(send_one "$SPAM_TX")" = blocked ] && spam_blocked=$((spam_blocked+1)) || spam_reached=$((spam_reached+1))
done
t1=$(python3 -c 'import time;print(time.time())')
spam_ms=$(python3 -c "print(f'{($t1-$t0)*1000:.0f}')")

# ── Fire legitimate traffic ─────────────────────────────────────────────────
banner "LEGITIMATE TRAFFIC — $LEGIT txs, paid priority fee"
legit_blocked=0; legit_reached=0
for _ in $(seq 1 "$LEGIT"); do
  [ "$(send_one "$LEGIT_TX")" = blocked ] && legit_blocked=$((legit_blocked+1)) || legit_reached=$((legit_reached+1))
done

# ── Result ──────────────────────────────────────────────────────────────────
banner "RESULT"
row "class" "sent" "blocked @ proxy" "reached node"
row "spam"  "$SPAM"  "$spam_blocked"  "$spam_reached"
row "legit" "$LEGIT" "$legit_blocked" "$legit_reached"

cu_saved=$(( spam_blocked * 500000 ))
echo
# Real inspection cost comes from the histogram (sum/count). The burst's wall time is
# dominated by spawning one curl per request in this shell loop, not by Aegis.
mean_us=$(curl -s "$PROXY/metrics" | awk '
  /^aegis_inspection_duration_microseconds_sum/   { sum=$2 }
  /^aegis_inspection_duration_microseconds_count/ { cnt=$2 }
  END { if (cnt>0) printf "%.1f", sum/cnt; else printf "n/a" }')
ok "all $SPAM spam txs answered locally in ${spam_ms} ms wall (shell-bound; no network hop)"
info "Aegis inspection cost per tx (parse + list + rule): ${mean_us} µs  ← from /metrics histogram"
info "compute the node was never asked to reserve: ${spam_blocked} × 500,000 CU = $(python3 -c "print(f'{$cu_saved:,}')") CU"
[ "$spam_reached" -eq 0 ] && ok "not one spam tx left the box" || info "$spam_reached spam tx(s) leaked (rule disabled?)"
[ "$legit_blocked" -eq 0 ] && ok "no legitimate traffic was blocked (no false positives)" || info "$legit_blocked legit tx(s) blocked"

echo
info "Edit the policy live: append a line to blocklist.txt and SIGHUP, or swap the Wasm rule — see demo/rehearse.sh."
info "Rule used: cu_requested > 400000 AND priority_fee == 0  → drop."

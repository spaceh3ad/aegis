#!/usr/bin/env bash
# Aegis demo — one script, three modes. No Docker, no emulated validator.
#
# Default: boots the release proxy in front of a real RPC and walks the full story with a
# pass/fail gate. Point at ANY RPC via AEGIS_RPC_URL / HELIUS_API_KEY — only the URL changes.
#
#   bash demo/demo.sh                     # full story against public devnet
#   AEGIS_RPC_URL="https://.../..." bash demo/demo.sh
#   HELIUS_API_KEY=xxx bash demo/demo.sh
#   SPAM=500 bash demo/demo.sh            # bigger spam burst (0 to skip)
#   PROXY=http://127.0.0.1:8899 bash demo/demo.sh   # target an already-running proxy
#                                                   # (e.g. the Docker stack) — read-only checks
#   WALLET=1 bash demo/demo.sh            # interactive: point Phantom/MetaMask here
#   INTERACTIVE=1 bash demo/demo.sh       # pause between beats for narration
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ── Config ──────────────────────────────────────────────────────────────────
PORT="${PORT:-8899}"
EXTERNAL="${PROXY:-}"                 # non-empty => target this, do not boot
WALLET="${WALLET:-0}"
SPAM="${SPAM:-100}"

if [ -n "${AEGIS_RPC_URL:-}" ]; then RPC="$AEGIS_RPC_URL"
elif [ -n "${HELIUS_API_KEY:-}" ]; then RPC="https://testnet.helius-rpc.com/?api-key=${HELIUS_API_KEY}"
else RPC="https://api.devnet.solana.com"; fi

BIN="$ROOT/target/release/aegis-proxy"
WORK="$(mktemp -d)"
BL="$WORK/blocklist.txt"
RULE="$WORK/rule.wasm"
ZFF="$ROOT/plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm"
APG="$ROOT/plugins/approve-guard/target/wasm32-wasip1/release/approve_guard.wasm"

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; M='\033[0;35m'; X='\033[0m'
banner(){ echo -e "\n${B}${C}━━━ $* ━━━${X}"; }
ok(){     echo -e "  ${G}✓${X} $*"; }
bad(){    echo -e "  ${R}✗${X} $*"; }
info(){   echo -e "  ${Y}•${X} $*"; }
blocked(){ echo -e "  ${R}[BLOCKED @ PROXY]${X} $*"; }
pause(){ if [ "${INTERACTIVE:-0}" = "1" ]; then echo -e "  ${M}▸ press enter${X}"; read -r _; fi; }

PASS=0; FAIL=0
# Aegis rejections always carry error.data.rule_id; downstream node errors do not.
expect_blocked(){ if [ -n "$2" ]; then blocked "$3 → rule=$2"; PASS=$((PASS+1)); else bad "$3 → expected block, got code=$1 (no rule_id)"; FAIL=$((FAIL+1)); fi; }
expect_allowed(){ if [ -z "$2" ]; then ok "$3 → passed inspection"; PASS=$((PASS+1)); else bad "$3 → wrongly blocked by $2"; FAIL=$((FAIL+1)); fi; }

PID=""
cleanup(){ [ -n "$PID" ] && { kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; }; rm -rf "$WORK"; }
trap cleanup EXIT INT TERM
set +m

# ── Build ───────────────────────────────────────────────────────────────────
if [ ! -x "$BIN" ]; then info "Building release binary..."; cargo build --release -p aegis-proxy >/dev/null 2>&1; fi
for p in zero-fee-filter approve-guard; do
    w="$ROOT/plugins/$p/target/wasm32-wasip1/release/$p.wasm"
    [ -f "$w" ] || (cd "$ROOT/plugins/$p" && cargo build --release --target wasm32-wasip1 >/dev/null 2>&1)
done

# ── Boot (unless targeting an external proxy) ───────────────────────────────
if [ -n "$EXTERNAL" ]; then
    PROXY="$EXTERNAL"
else
    PROXY="http://127.0.0.1:${PORT}"
    cp "$ROOT/blocklist.txt" "$BL"; cp "$ZFF" "$RULE"
    DOWNSTREAM_URL="$RPC" LISTEN_ADDR="127.0.0.1:${PORT}" BLOCKLIST_PATH="$BL" WASM_PLUGIN_PATH="$RULE" \
        "$BIN" >"$WORK/aegis.log" 2>&1 &
    PID=$!
fi
for _ in $(seq 1 40); do curl -sf "$PROXY/metrics" >/dev/null 2>&1 && break; sleep 0.25; done
curl -sf "$PROXY/metrics" >/dev/null 2>&1 || { bad "proxy not reachable at $PROXY"; [ -f "$WORK/aegis.log" ] && tail -5 "$WORK/aegis.log"; exit 1; }

# ── tx builders ─────────────────────────────────────────────────────────────
sol_tx(){ python3 - "$1" <<'PY'
import sys, base64
k=bytes.fromhex(sys.argv[1]); assert len(k)==32
tx=b'\x01'+b'\x00'*64+b'\x80'+b'\x01\x00\x00'+b'\x01'+k+b'\x00'*32+b'\x00'+b'\x00'
print(base64.b64encode(tx).decode())
PY
}
cu_tx(){ # Solana tx: 500k CU, zero price -> spam
python3 - <<'PY'
import base64, base58
cb=base58.b58decode("ComputeBudget111111111111111111111111111111")
def u16(n):
    o=bytearray()
    while True:
        b=n&0x7f; n>>=7
        if n: o.append(b|0x80)
        else: o.append(b); break
    return bytes(o)
data=bytes([2])+(500000).to_bytes(4,'little')
hdr=bytes([1,0,1]); accts=u16(2)+bytes(32)+cb; bh=bytes(32)
msg=hdr+accts+bh+u16(1)+bytes([1])+u16(0)+u16(len(data))+data
print(base64.b64encode(u16(1)+bytes(64)+msg).decode())
PY
}
evm_tx(){ # $1 selector hex, $2 nonce
python3 - "$1" "$2" <<'PY'
import sys
from eth_account import Account
acct=Account.from_key("0x"+"11"*32)
data="0x"+sys.argv[1]+"33"*32+"ff"*32
tx={"nonce":int(sys.argv[2]),"gasPrice":10**9,"gas":60000,"to":"0x"+"22"*20,"value":0,"data":data,"chainId":1}
print(acct.sign_transaction(tx).rawTransaction.hex())
PY
}
b58(){ python3 -c "import base58;print(base58.b58encode(bytes.fromhex('$1')).decode())"; }
len_list(){ grep -v '^[[:space:]]*#' "$BL" 2>/dev/null | grep -cv '^[[:space:]]*$'; }
send_tx(){ # $1 method, $2 param, $3 extra-json, $4 proxy(optional) -> "code rule"
  local p="${4:-$PROXY}"
  curl -s -m 8 -X POST "$p" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":[\"$2\"$3]}" \
  | python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id',''))"
}

BLOCKED_KEY="CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv"
BLOCK_HEX="$(python3 -c "import base58;print(base58.b58decode('$BLOCKED_KEY').hex())")"

echo -e "\n${B}${M}AEGIS${X} ${B}— a programmable, wire-speed transaction firewall for RPC infrastructure${X}"
info "endpoint : $RPC"
info "proxy    : $PROXY"
[ -n "$PID" ] && info "pid $PID"

# ═══ WALLET MODE ════════════════════════════════════════════════════════════
if [ "$WALLET" = "1" ]; then
    banner "WALLET DEMO — point Phantom/MetaMask here"
    info "A wallet broadcasts via its network's RPC, so put Aegis in the middle by making"
    info "it the wallet's RPC endpoint (not an interceptor). localhost is fine."
    echo
    cat <<EOF
   Phantom : Settings → Developer Settings → enable testnet → Solana Devnet RPC URL =
             $PROXY
   MetaMask: Settings → Networks → Base Sepolia → RPC URL = $PROXY   (CHAIN=base upstream)

   Then send a tiny amount TO:  $BLOCKED_KEY
   Aegis refuses it at the edge; then send to any normal address — it passes.
EOF
    echo
    banner "LIVE — waiting for wallet transactions (Ctrl-C to stop)"
    prev=-1
    while kill -0 "$PID" 2>/dev/null 2>&1 || [ -n "$EXTERNAL" ]; do
        m=$(curl -s "$PROXY/metrics" 2>/dev/null)
        bl=$(printf '%s' "$m" | awk '/^aegis_transactions_blocked_total/{s+=$2} END{print s+0}')
        al=$(printf '%s' "$m" | awk '/^aegis_transactions_total\{verdict="allow"\}/{s+=$2} END{print s+0}')
        if [ "$bl" != "$prev" ]; then
            echo -e "  ${R}⛔ BLOCKED${X} total=$bl  allowed=$al"
            prev="$bl"
        fi
        sleep 1
    done
    exit 0
fi

# ═══ BEAT 1 — live reads pass through ═══════════════════════════════════════
banner "BEAT 1 — Real chain data flows through untouched"
for m in getVersion getHealth getSlot; do
    r=$(curl -s -m 10 -X POST "$PROXY" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$m\"}")
    if echo "$r" | grep -q '"result"'; then ok "$m → $(echo "$r" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["result"])[:64])' 2>/dev/null)"; PASS=$((PASS+1)); else bad "$m → $r"; FAIL=$((FAIL+1)); fi
done
info "Only transaction submission is inspected; the read path is invisible."
pause

# ═══ BEAT 2 — static list block ═════════════════════════════════════════════
banner "BEAT 2 — Blocklisted signer rejected before the RPC sees it"
read -r c r < <(send_tx sendTransaction "$(sol_tx "$BLOCK_HEX")" ',{"encoding":"base64"}')
expect_blocked "$c" "$r" "sendTransaction signed by a listed address"
info "Rejection is local: parse → O(1) lookup → JSON-RPC error. The tx never leaves the box."
pause

# ═══ BEAT 3 — clean passthrough ═════════════════════════════════════════════
banner "BEAT 3 — A clean transaction is forwarded"
CLEAN=$(python3 -c "import os;print(os.urandom(32).hex())")
read -r c r < <(send_tx sendTransaction "$(sol_tx "$CLEAN")" ',{"encoding":"base64"}')
expect_allowed "$c" "$r" "unlisted signer"
pause

# External-proxy mode cannot control the process or its rule; stop here.
if [ -n "$EXTERNAL" ]; then
    banner "RESULT"
    echo -e "  ${G}passed: $PASS${X}   ${R}failed: $FAIL${X}"
    info "external proxy: hot-reload / rule-swap beats need process control — run without PROXY for those."
    [ "$FAIL" -eq 0 ]
    exit
fi

# ═══ BEAT 4 — SIGHUP hot reload ═════════════════════════════════════════════
banner "BEAT 4 — New threat intel live in <1s, no restart"
NEWKEY=$(python3 -c "import os;print(os.urandom(32).hex())")
read -r c r < <(send_tx sendTransaction "$(sol_tx "$NEWKEY")" ',{"encoding":"base64"}')
info "Before: a fresh, unlisted signer ..."
expect_allowed "$c" "$r" "  (sent before it was listed)"
echo "$(b58 "$NEWKEY")" >> "$BL"; kill -HUP "$PID"; sleep 0.6
info "  → appended to blocklist.txt and sent SIGHUP"
read -r c r < <(send_tx sendTransaction "$(sol_tx "$NEWKEY")" ',{"encoding":"base64"}')
expect_blocked "$c" "$r" "same transaction, after hot reload"
ok "list is now $(len_list) entries — swapped atomically, zero dropped connections"
pause

# ═══ BEAT 5 — Wasm rule (Solana) ════════════════════════════════════════════
banner "BEAT 5 — Programmable policy: Solana spam filter (Wasm)"
read -r c r < <(send_tx sendTransaction "$(cu_tx)" ',{"encoding":"base64"}')
expect_blocked "$c" "$r" "500k-CU transaction, zero priority fee"
info "Rule: cu_requested > 400k AND priority_fee == 0  → drop spam before it burns a slot."
pause

# ═══ BEAT 6 — spam burst (volume) ═══════════════════════════════════════════
if [ "$SPAM" -gt 0 ]; then
    banner "BEAT 6 — Spam is a volume problem ($SPAM txs)"
    STX="$(cu_tx)"; LTX="$(python3 - <<'PY'
import base64, base58
cb=base58.b58decode("ComputeBudget111111111111111111111111111111")
def u16(n):
    o=bytearray()
    while True:
        b=n&0x7f; n>>=7
        if n: o.append(b|0x80)
        else: o.append(b); break
    return bytes(o)
def instr(idx,d): return bytes([idx])+u16(0)+u16(len(d))+d
ixs=[instr(1,bytes([2])+(200000).to_bytes(4,'little')), instr(1,bytes([3])+(1000).to_bytes(8,'little'))]
hdr=bytes([1,0,1]); accts=u16(2)+bytes(32)+cb; bh=bytes(32)
msg=hdr+accts+bh+u16(len(ixs))+b''.join(ixs)
print(base64.b64encode(u16(1)+bytes(64)+msg).decode())
PY
)"
    verdict(){ curl -s -m 8 -X POST "$PROXY" -H 'content-type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"sendTransaction\",\"params\":[\"$1\",{\"encoding\":\"base64\"}]}" \
      | python3 -c "import sys,json;print('blocked' if ((json.load(sys.stdin).get('error') or {}).get('data') or {}).get('rule_id') else 'reached')" 2>/dev/null || echo reached; }
    sbl=0; sr=0; info "firing $SPAM zero-fee spam txs..."
    for _ in $(seq 1 "$SPAM"); do [ "$(verdict "$STX")" = blocked ] && sbl=$((sbl+1)) || sr=$((sr+1)); done
    lbl=0; lr=0
    for _ in $(seq 1 10); do [ "$(verdict "$LTX")" = blocked ] && lbl=$((lbl+1)) || lr=$((lr+1)); done
    ok "spam: $sbl blocked, $sr forwarded   |   paid traffic: $lbl blocked, $lr forwarded"
    info "compute the node never reserved: $sbl × 500,000 CU = $(python3 -c "print(f'{$((sbl*500000)):,}')") CU"
    [ "$sr" -eq 0 ] && [ "$lbl" -eq 0 ] && { ok "no spam leaked, no false positives"; PASS=$((PASS+1)); } || FAIL=$((FAIL+1))
fi

# ═══ BEAT 7 — cross-chain rule hot-swap ═════════════════════════════════════
banner "BEAT 7 — Swap the rule to an EVM guard, live"
info "replacing the Wasm rule with approve-guard, then SIGHUP (no restart)..."
cp "$APG" "$RULE"; kill -HUP "$PID"; sleep 0.6
read -r c r < <(send_tx eth_sendRawTransaction "$(evm_tx 095ea7b3 0)" "")
expect_blocked "$c" "$r" "ERC-20 approve(address,uint256)"
read -r c r < <(send_tx eth_sendRawTransaction "$(evm_tx a9059cbb 1)" "")
expect_allowed "$c" "$r" "ERC-20 transfer(address,uint256)"
info "Same process, same port — a Solana spam filter became an EVM approval guard."
info "(EVM selector screening shown here; full EVM delivery is covered by the integration tests.)"
pause

# ═══ RESULT ═════════════════════════════════════════════════════════════════
banner "RESULT"
echo -e "  ${G}passed: $PASS${X}   ${R}failed: $FAIL${X}"
[ "$FAIL" -eq 0 ] && echo -e "  ${G}${B}All beats verified.${X}" || echo -e "  ${R}${B}Some beats failed — see above.${X}"
info "proxy log: $WORK/aegis.log (removed on exit)"
[ "$FAIL" -eq 0 ]

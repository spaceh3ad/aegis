#!/usr/bin/env bash
# Aegis — one-take demo. No Docker, no emulated validator, deterministic.
#
# Presents the full pitch in six beats:
#   1. Latency      — sub-microsecond inspection (real Criterion numbers)
#   2. Live reads   — real chain data flows through the firewall untouched
#   3. Static list  — a blocklisted signer is rejected before the RPC sees it
#   4. Hot reload   — the SAME tx flips from allowed to blocked on SIGHUP, no restart
#   5. Wasm rule    — programmable policy drops Solana spam (-32002)
#   6. Cross-chain  — hot-swap the rule to an EVM guard; approve() blocked, transfer() passes
#
# Usage:
#   bash demo/rehearse.sh                 # public testnet; cites committed latency numbers
#   HELIUS_API_KEY=xxx bash demo/rehearse.sh
#   AEGIS_RPC_URL="https://devnet.helius-rpc.com/?api-key=$KEY" bash demo/rehearse.sh
#   BENCH=1 bash demo/rehearse.sh         # re-run Criterion live (opt-in; use an idle machine)
#   INTERACTIVE=1 bash demo/rehearse.sh   # pause for narration between beats
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ── Config ──────────────────────────────────────────────────────────────────
if [ -n "${AEGIS_RPC_URL:-}" ]; then RPC="$AEGIS_RPC_URL"
elif [ -n "${HELIUS_API_KEY:-}" ]; then RPC="https://testnet.helius-rpc.com/?api-key=${HELIUS_API_KEY}"
else RPC="https://api.testnet.solana.com"; fi

PORT="${AEGIS_LISTEN_PORT:-18899}"
PROXY="http://127.0.0.1:${PORT}"
BIN="$ROOT/target/release/aegis-proxy"
WORK="$(mktemp -d)"
BL="$WORK/blocklist.txt"
RULE="$WORK/rule.wasm"

# Colors
R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; M='\033[0;35m'; X='\033[0m'
banner(){ echo -e "\n${B}${C}━━━ $* ━━━${X}"; }
ok(){     echo -e "  ${G}✓${X} $*"; }
bad(){    echo -e "  ${R}✗${X} $*"; }
info(){   echo -e "  ${Y}•${X} $*"; }
blocked(){ echo -e "  ${R}[BLOCKED @ PROXY]${X} $*"; }
pause(){ if [ "${INTERACTIVE:-0}" = "1" ]; then echo -e "  ${M}▸ press enter${X}"; read -r _; fi; }

PASS=0; FAIL=0
# Aegis rejections always carry error.data.rule_id; downstream node errors do not.
# That is the authoritative discriminator (JSON-RPC codes alone can collide with the chain).
expect_blocked(){ # $1=code $2=rule $3=label
    if [ -n "$2" ]; then
        blocked "$3 → code=$1 rule=$2"; PASS=$((PASS+1))
    else bad "$3 → expected an Aegis block, got code=$1 (no rule_id)"; FAIL=$((FAIL+1)); fi
}
expect_allowed(){ # $1=code $2=rule $3=label
    if [ -z "$2" ]; then
        ok "$3 → passed inspection (Aegis did not block; node replied code=$1)"; PASS=$((PASS+1))
    else bad "$3 → wrongly blocked by rule $2"; FAIL=$((FAIL+1)); fi
}

cleanup(){ if [ -n "${PID:-}" ]; then kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; fi; rm -rf "$WORK"; }
trap cleanup EXIT
# Suppress shell job-control noise on teardown.
set +m

# ── Build ───────────────────────────────────────────────────────────────────
if [ ! -x "$BIN" ]; then info "Building release binary..."; cargo build --release -p aegis-proxy >/dev/null 2>&1; fi
# Ensure both plugins are built so we can hot-swap between them.
for p in zero-fee-filter approve-guard; do
    w="$ROOT/plugins/$p/target/wasm32-wasip1/release/$p.wasm"
    if [ ! -f "$w" ]; then (cd "$ROOT/plugins/$p" && cargo build --release --target wasm32-wasip1 >/dev/null 2>&1); fi
done
ZFF="$ROOT/plugins/zero-fee-filter/target/wasm32-wasip1/release/zero_fee_filter.wasm"
APG="$ROOT/plugins/approve-guard/target/wasm32-wasip1/release/approve_guard.wasm"

# ── Boot ────────────────────────────────────────────────────────────────────
cp "$ROOT/blocklist.txt" "$BL"
cp "$ZFF" "$RULE"
info "RPC endpoint : $RPC"
info "Proxy        : $PROXY  (pid pending)"
DOWNSTREAM_URL="$RPC" LISTEN_ADDR="127.0.0.1:${PORT}" BLOCKLIST_PATH="$BL" WASM_PLUGIN_PATH="$RULE" \
    "$BIN" >"$WORK/aegis.log" 2>&1 &
PID=$!
for _ in $(seq 1 40); do curl -sf "$PROXY/metrics" >/dev/null 2>&1 && break; sleep 0.25; done
curl -sf "$PROXY/metrics" >/dev/null 2>&1 || { echo -e "${R}proxy failed to start:${X}"; tail -5 "$WORK/aegis.log"; exit 1; }

# ── tx builders ─────────────────────────────────────────────────────────────
sol_tx(){ # $1 = 32-byte pubkey hex -> base64 v0 tx carrying that key as signer
python3 - "$1" <<'PY'
import sys, base64
k=bytes.fromhex(sys.argv[1]); assert len(k)==32
tx=b'\x01'+b'\x00'*64+b'\x80'+b'\x01\x00\x00'+b'\x01'+k+b'\x00'*32+b'\x00'+b'\x00'
print(base64.b64encode(tx).decode())
PY
}
cu_tx(){ # Solana tx: ComputeBudget SetComputeUnitLimit(500000), zero price -> spam
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
hdr=bytes([1,0,1]); accts=u16(2)+bytes(32)+cb; bh=bytes(32)
data=bytes([2])+(500000).to_bytes(4,'little')
instr=bytes([1])+u16(0)+u16(len(data))+data
msg=hdr+accts+bh+u16(1)+instr
print(base64.b64encode(u16(1)+bytes(64)+msg).decode())
PY
}
evm_tx(){ # $1 = 4-byte selector hex, $2 = nonce
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
# count real entries (non-blank, non-comment)
len_list(){ grep -v '^[[:space:]]*#' "$BL" | grep -cv '^[[:space:]]*$'; }
# sends a json-rpc call, prints "code rule" (code=none when allowed)
send_tx(){ # $1=method $2=param $3=extra-json-or-empty
curl -s -m 8 -X POST "$PROXY" -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":[\"$2\"$3]}" \
| python3 -c "import sys,json
d=json.load(sys.stdin); e=d.get('error') or {}
print(e.get('code','none'), (e.get('data') or {}).get('rule_id',''))"
}

echo -e "\n${B}${M}AEGIS${X} ${B}— a programmable, wire-speed transaction firewall for RPC infrastructure${X}"
info "pid $PID  |  plugin: zero-fee-filter  |  list: $(len_list) entries"

# ═══ BEAT 1 — Latency ═══════════════════════════════════════════════════════
banner "BEAT 1 — Inspection latency (committed Criterion numbers)"
# Default: cite the numbers committed in README.md. A demo run must NOT re-benchmark:
# a bench is slow (~30s), compiles the bench dep-tree, and is contention-sensitive — on a
# busy box Criterion reports a spurious "regression". Opt in with BENCH=1 to reproduce live.
if [ "${BENCH:-0}" = "1" ]; then
    info "BENCH=1 — running a short Criterion pass (idle machine recommended)..."
    for name in "full_pipeline/allow" "blocklist_50k_entries/allow"; do
        out=$(cargo bench -p aegis-bench -- "$name" \
              --warm-up-time 0.5 --measurement-time 1 --sample-size 10 2>&1 | \
              awk -v n="$name" '
                index($0,n)==1 {
                  if ($0 ~ /time:/) { sub(/.*time:[[:space:]]*/,""); print; exit }
                  getline; while ($0 !~ /time:/) getline
                  sub(/.*time:[[:space:]]*/,""); print; exit
                }')
        if [ -n "$out" ]; then ok "$(printf '%-28s' "$name") $out"; else info "$name (no sample captured)"; fi
    done
else
    info "full pipeline (parse + policy)   ~700 ns"
    info "50,000-entry list lookup         ~18 ns   (same as 1 entry: O(1))"
    info "source: README.md, measured on an idle Apple M2 Pro — re-run live with BENCH=1"
fi
info "O(1): a 50,000-entry list costs the same as one entry."
pause

# ═══ BEAT 2 — Live reads pass through ═══════════════════════════════════════
banner "BEAT 2 — Real chain data flows through untouched"
for m in getVersion getHealth getSlot; do
    r=$(curl -s -m 10 -X POST "$PROXY" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$m\"}")
    if echo "$r" | grep -q '"result"'; then ok "$m → $(echo "$r" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["result"])[:70])' 2>/dev/null)"; PASS=$((PASS+1))
    else bad "$m → $r"; FAIL=$((FAIL+1)); fi
done
info "The read path is invisible. Only transaction submission is inspected."
pause

# ═══ BEAT 3 — Static list block ═════════════════════════════════════════════
banner "BEAT 3 — Blocklisted signer rejected before the RPC sees it"
BLOCKED_KEY="CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv"
read -r c0 r0 < <(send_tx sendTransaction "$(sol_tx "$(python3 -c "import base58;print(base58.b58decode('$BLOCKED_KEY').hex())")")" ',{"encoding":"base64"}')
expect_blocked "$c0" "$r0" "sendTransaction signed by a listed address"
info "Rejection is local: parse → O(1) list lookup → JSON-RPC error. The tx never leaves the box."
pause

# ═══ BEAT 4 — Hot reload (the wow) ══════════════════════════════════════════
banner "BEAT 4 — New threat intel live in <1s, no restart"
NEWKEY=$(python3 -c "import os;print(os.urandom(32).hex())")
read -r c1 r1 < <(send_tx sendTransaction "$(sol_tx "$NEWKEY")" ',{"encoding":"base64"}')
info "Before: a fresh, unlisted signer ..."
expect_allowed "$c1" "$r1" "  (sent before it was listed)"
echo "$(b58 "$NEWKEY")" >> "$BL"
kill -HUP "$PID"; sleep 0.6
info "  → appended the key to blocklist.txt and sent SIGHUP"
read -r c2 r2 < <(send_tx sendTransaction "$(sol_tx "$NEWKEY")" ',{"encoding":"base64"}')
expect_blocked "$c2" "$r2" "same transaction, after hot reload"
ok "list is now $(len_list) entries — swapped atomically, zero dropped connections"
pause

# ═══ BEAT 5 — Wasm rule (Solana) ════════════════════════════════════════════
banner "BEAT 5 — Programmable policy: Solana spam filter (Wasm)"
read -r c3 r3 < <(send_tx sendTransaction "$(cu_tx)" ',{"encoding":"base64"}')
expect_blocked "$c3" "$r3" "500k-CU transaction with zero priority fee"
info "Rule: cu_requested > 400k AND priority_fee == 0  → drop the spam before it burns a slot."
pause

# ═══ BEAT 6 — Cross-chain rule hot-swap ═════════════════════════════════════
banner "BEAT 6 — Swap the rule to an EVM guard, live"
info "replacing the Wasm rule with approve-guard, then SIGHUP (no restart)..."
cp "$APG" "$RULE"; kill -HUP "$PID"; sleep 0.6
read -r c4 r4 < <(send_tx eth_sendRawTransaction "$(evm_tx 095ea7b3 0)" "")
expect_blocked "$c4" "$r4" "ERC-20 approve(address,uint256) — a drainer entry point"
read -r c5 r5 < <(send_tx eth_sendRawTransaction "$(evm_tx a9059cbb 1)" "")
expect_allowed "$c5" "$r5" "  ERC-20 transfer(address,uint256)"
info "Same process, same port — Solana spam filter became an EVM approval guard."

# ═══ SUMMARY ════════════════════════════════════════════════════════════════
banner "RESULT"
echo -e "  ${G}passed: $PASS${X}   ${R}failed: $FAIL${X}"
[ "$FAIL" -eq 0 ] && echo -e "  ${G}${B}All beats verified.${X}" || echo -e "  ${R}${B}Some beats failed — see above.${X}"
info "proxy log: $WORK/aegis.log (removed on exit)"
[ "$FAIL" -eq 0 ]

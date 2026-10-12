#!/usr/bin/env bash
# Aegis wallet demo — click through a real wallet (Phantom / MetaMask).
#
# A wallet broadcasts through the RPC configured for its network, so "putting Aegis in
# the middle" means pointing the wallet's RPC URL at Aegis. This script boots Aegis in
# front of a real cluster and prints the exact settings to paste in. localhost is fine —
# Phantom and MetaMask both accept http://127.0.0.1:<port> as a custom RPC. Set TUNNEL=1
# only if a wallet refuses a non-https URL.
#
# Then you send a transaction TO a blocklisted address from the wallet. Aegis rejects it
# at the edge (-32001) and the wallet surfaces the error — the tx never reaches the chain.
#
# Usage:
#   BLOCK_ADDR=<base58> bash demo/wallet-demo.sh                # Solana devnet (Phantom)
#   CHAIN=base BLOCK_ADDR=0x... bash demo/wallet-demo.sh        # Base Sepolia (EVM)
#   TUNNEL=1 BLOCK_ADDR=... bash demo/wallet-demo.sh            # only if https is required
#   RPC=<custom> PORT=8899 bash demo/wallet-demo.sh
#
# Ctrl-C to stop.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CHAIN="${CHAIN:-solana}"
PORT="${PORT:-8899}"
TUNNEL="${TUNNEL:-0}"
BIN="$ROOT/target/release/aegis-proxy"
WORK="$(mktemp -d)"
BL="$WORK/blocklist.txt"

R='\033[0;31m'; G='\033[0;32m'; Y='\033[1;33m'; B='\033[1m'; C='\033[0;36m'; M='\033[0;35m'; X='\033[0m'
banner(){ echo -e "\n${B}${C}━━━ $* ━━━${X}"; }
ok(){     echo -e "  ${G}✓${X} $*"; }
info(){   echo -e "  ${Y}•${X} $*"; }
warn(){   echo -e "  ${R}!${X} $*"; }

# ── Pick the downstream RPC and a demo address to block ─────────────────────
case "$CHAIN" in
  solana) DEFAULT_RPC="https://api.devnet.solana.com"; NET_NAME="Solana Devnet" ;;
  base)   DEFAULT_RPC="https://sepolia.base.org";      NET_NAME="Base Sepolia" ;;
  eth)    DEFAULT_RPC="https://rpc.sepolia.org";       NET_NAME="Sepolia" ;;
  *)      DEFAULT_RPC="${RPC:-}";                      NET_NAME="$CHAIN" ;;
esac
RPC="${RPC:-$DEFAULT_RPC}"

# Default demo address: the Solana key already shipped in blocklist.txt. Override for EVM.
if [ -z "${BLOCK_ADDR:-}" ]; then
  if [ "$CHAIN" = "solana" ]; then BLOCK_ADDR="CVC5aBX8ckVF2ByAKRXjg9gPMqYNNCSBFkSRuGvxLbzv"
  else BLOCK_ADDR="0x000000000000000000000000000000000000dEaD"; fi
fi

# ── Build ───────────────────────────────────────────────────────────────────
if [ ! -x "$BIN" ]; then info "Building release binary..."; cargo build --release -p aegis-proxy >/dev/null 2>&1; fi

# Working blocklist = repo list + the demo address (deduped, keep it deterministic).
cp "$ROOT/blocklist.txt" "$BL"
grep -qxF "$BLOCK_ADDR" "$BL" || echo "$BLOCK_ADDR" >> "$BL"

# ── Boot Aegis ──────────────────────────────────────────────────────────────
DOWNSTREAM_URL="$RPC" LISTEN_ADDR="127.0.0.1:${PORT}" BLOCKLIST_PATH="$BL" \
    "$BIN" >"$WORK/aegis.log" 2>&1 &
PID=$!

TUNNEL_PID=""
cleanup(){
  [ -n "$TUNNEL_PID" ] && kill "$TUNNEL_PID" 2>/dev/null || true
  kill "$PID" 2>/dev/null || true
  rm -rf "$WORK"
  echo; info "Aegis stopped."
}
trap cleanup EXIT INT TERM

for _ in $(seq 1 40); do curl -sf "http://127.0.0.1:${PORT}/metrics" >/dev/null 2>&1 && break; sleep 0.25; done
curl -sf "http://127.0.0.1:${PORT}/metrics" >/dev/null 2>&1 || { warn "Aegis failed to start:"; tail -5 "$WORK/aegis.log"; exit 1; }

# ── Optional HTTPS tunnel ────────────────────────────────────────────────────
URL="http://127.0.0.1:${PORT}"
if [ "$TUNNEL" = "1" ]; then
  if ! command -v cloudflared >/dev/null 2>&1; then
    warn "cloudflared not found; staying on http. Install it or use a manual tunnel."
  else
    info "opening cloudflared quick tunnel (https)..."
    cloudflared tunnel --url "http://127.0.0.1:${PORT}" --no-autoupdate >"$WORK/tunnel.log" 2>&1 &
    TUNNEL_PID=$!
    for _ in $(seq 1 40); do
      URL=$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "$WORK/tunnel.log" | head -1)
      [ -n "$URL" ] && break
      sleep 0.5
    done
    [ -n "$URL" ] && ok "public endpoint: $URL" || { warn "tunnel URL not captured; see $WORK/tunnel.log — staying on http"; URL="http://127.0.0.1:${PORT}"; }
  fi
fi

# ── Instructions ─────────────────────────────────────────────────────────────
banner "AEGIS WALLET DEMO — $NET_NAME"
info "Aegis     : $URL  (this is your wallet's RPC)"
info "downstream: $RPC"
info "blocked   : ${B}${BLOCK_ADDR}${X}"
echo
echo -e "  ${B}Wallet setup${X}"
if [ "$CHAIN" = "solana" ]; then
  cat <<EOF
   Phantom → Settings → Developer Settings → enable testnet, then set the
   ${NET_NAME} RPC URL to:   ${URL}

   (If the field rejects an http URL, re-run with TUNNEL=1 for an https URL.)
EOF
else
  cat <<EOF
   MetaMask → Settings → Networks → ${NET_NAME} → Edit → RPC URL =
     ${URL}
   Save, then select the ${NET_NAME} network.
EOF
fi
echo
echo -e "  ${B}The demo${X}"
cat <<EOF
   1. In the wallet, send a tiny amount TO:  ${BLOCK_ADDR}
   2. Aegis rejects it at the edge (JSON-RPC error, rule_id=blocklist:file).
      The wallet shows the failure; the transaction never reaches the chain.
   3. Then send to any normal address — it passes through untouched.

   Watch this terminal: every block is logged below as it happens.
EOF
echo

# ── Live watch ───────────────────────────────────────────────────────────────
banner "LIVE — waiting for wallet transactions (Ctrl-C to stop)"
prev=0
while kill -0 "$PID" 2>/dev/null; do
  m=$(curl -s "http://127.0.0.1:${PORT}/metrics" 2>/dev/null)
  blocked=$(printf '%s' "$m" | awk '/^aegis_transactions_blocked_total/{s+=$2} END{print s+0}')
  allowed=$(printf '%s' "$m" | awk '/^aegis_transactions_total\{verdict="allow"\}/{s+=$2} END{print s+0}')
  if [ "$blocked" != "$prev" ]; then
    rule=$(printf '%s' "$m" | awk -F'"' '/^aegis_transactions_blocked_total/{print $2; exit}')
    echo -e "  ${R}⛔ BLOCKED${X}  total=$blocked  (rule=$rule)   allowed=$allowed"
    prev="$blocked"
  fi
  sleep 1
done

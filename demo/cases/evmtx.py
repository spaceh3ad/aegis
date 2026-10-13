#!/usr/bin/env python3
"""Sign EVM transactions with anvil account #0 (chain-id 31337) for the Aegis demo.

Usage:
  evmtx.py approve      <token> <spender>   <amount> <nonce>
  evmtx.py transfer     <token> <to>        <amount> <nonce>
  evmtx.py transferfrom <token> <from> <to> <amount> <nonce>
  evmtx.py raw          <token> <selector>  <nonce>
"""
import sys

from eth_account import Account

KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
CHAIN_ID = 31337
MAX_UINT = 2**256 - 1


def word(x) -> str:
    if isinstance(x, str):
        x = int(x, 16) if x.startswith("0x") else int(x)
    return x.to_bytes(32, "big").hex()


def addr(a: str) -> str:
    return a[2:].lower().rjust(64, "0")


def build(to: str, data_hex: str, nonce: int, gas: int = 200_000) -> str:
    acct = Account.from_key(KEY)
    tx = {
        "nonce": nonce,
        "gasPrice": 10**9,
        "gas": gas,
        "to": to,
        "value": 0,
        "data": "0x" + data_hex,
        "chainId": CHAIN_ID,
    }
    raw = acct.sign_transaction(tx).rawTransaction
    raw_hex = raw.hex() if isinstance(raw, (bytes, bytearray)) else str(raw)
    return raw_hex if raw_hex.startswith("0x") else "0x" + raw_hex


def main() -> None:
    cmd, a = sys.argv[1], sys.argv[2:]
    if cmd == "approve":
        token, spender, amount, nonce = a[0], a[1], a[2], int(a[3])
        amt = "ff" * 32 if amount.lower() in ("max", "unlimited") else word(amount)
        print(build(token, "095ea7b3" + addr(spender) + amt, nonce))
    elif cmd == "transfer":
        token, to, amount, nonce = a[0], a[1], a[2], int(a[3])
        print(build(token, "a9059cbb" + addr(to) + word(amount), nonce))
    elif cmd == "transferfrom":
        token, frm, to, amount, nonce = a[0], a[1], a[2], a[3], int(a[4])
        print(build(token, "23b872dd" + addr(frm) + addr(to) + word(amount), nonce))
    elif cmd == "raw":
        token, selector, nonce = a[0], a[1].removeprefix("0x"), int(a[2])
        print(build(token, selector, nonce))
    else:
        sys.exit(f"unknown command {cmd}")


if __name__ == "__main__":
    main()

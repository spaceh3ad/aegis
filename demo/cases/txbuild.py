#!/usr/bin/env python3
"""Build base64 Solana transactions for the Aegis demo cases. No RPC, no signing.

Usage:
  txbuild.py simple <key_hex>
  txbuild.py spam <cu_limit> <cu_price>
  txbuild.py program <program_b58> [recipient_hex]
  txbuild.py lut <program_b58> <lut_b58>
"""
import base64
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # demo/
import b58

base58 = b58  # vendored pure-Python base58; no third-party dependency


def u16(n: int) -> bytes:
    o = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        o.append(b | 0x80 if n else b)
        if not n:
            break
    return bytes(o)


def _wrap_v0(msg: bytes) -> str:
    # v0 VersionedMessage in bincode: 1 zero signature, the 0x80 v0 prefix, then the message.
    return base64.b64encode(u16(1) + bytes(64) + b"\x80" + msg).decode()


def _wrap_legacy(msg: bytes) -> str:
    # Legacy VersionedMessage: no version prefix.
    return base64.b64encode(u16(1) + bytes(64) + msg).decode()


def simple(key_hex: str) -> str:
    """Minimal legacy tx: one static account key, no instructions."""
    k = bytes.fromhex(key_hex)
    assert len(k) == 32
    return _wrap_legacy(bytes([1, 0, 0]) + u16(1) + k + bytes(32) + u16(0) + u16(0))


def spam(cu_limit: int, cu_price: int) -> str:
    """ComputeBudget SetComputeUnitLimit (+ optional SetComputeUnitPrice).

    No LUTs, so legacy encoding (no 0x80) — v0 fails to deserialize.
    """
    cb = base58.b58decode("ComputeBudget111111111111111111111111111111")
    ixs = [bytes([1]) + u16(0) + u16(5) + bytes([2]) + cu_limit.to_bytes(4, "little")]
    if cu_price > 0:
        ixs.append(bytes([1]) + u16(0) + u16(9) + bytes([3]) + cu_price.to_bytes(8, "little"))
    hdr = bytes([1, 0, 1])
    accts = u16(2) + bytes(32) + cb
    msg = hdr + accts + bytes(32) + u16(len(ixs)) + b"".join(ixs)
    return _wrap_legacy(msg)


def program(program_b58: str, recipient_hex: str | None = None) -> str:
    """Invoke a program id, optionally passing a writable recipient account.

    No LUTs, so this is a legacy message (no 0x80 prefix) — sending it as v0 fails to
    deserialize.
    """
    prog = base58.b58decode(program_b58)
    payer = bytes(32)
    if recipient_hex:
        rec = bytes.fromhex(recipient_hex)
        keys = [payer, rec, prog]
        hdr = bytes([1, 0, 1])
        ix = bytes([2]) + u16(1) + bytes([1]) + u16(0)
    else:
        keys = [payer, prog]
        hdr = bytes([1, 0, 1])
        ix = bytes([1]) + u16(0) + u16(0)
    accts = u16(len(keys)) + b"".join(keys)
    msg = hdr + accts + bytes(32) + u16(1) + ix
    return _wrap_legacy(msg)


def lut(program_b58: str, lut_b58: str) -> str:
    """Invoke a program, passing an account loaded *through a lookup table*, v0."""
    prog = base58.b58decode(program_b58)
    lut_key = base58.b58decode(lut_b58)
    keys = [bytes(32), prog]
    hdr = bytes([1, 0, 1])
    accts = u16(2) + b"".join(keys)
    ix = bytes([1]) + u16(1) + bytes([2]) + u16(0)
    lookup = lut_key + u16(1) + bytes([0]) + u16(0)
    msg = hdr + accts + bytes(32) + u16(1) + ix + u16(1) + lookup
    return _wrap_v0(msg)


def main() -> None:
    cmd = sys.argv[1]
    if cmd == "simple":
        print(simple(sys.argv[2]))
    elif cmd == "spam":
        print(spam(int(sys.argv[2]), int(sys.argv[3])))
    elif cmd == "program":
        print(program(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else None))
    elif cmd == "lut":
        print(lut(sys.argv[2], sys.argv[3]))
    else:
        sys.exit(f"unknown command {cmd}")


if __name__ == "__main__":
    main()

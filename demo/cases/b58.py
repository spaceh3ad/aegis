"""Minimal pure-Python base58 (Bitcoin alphabet). No third-party deps.

So the demo's transaction builders run on ANY python3, regardless of which interpreter is
active or what is installed.
"""

_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_INDEX = {c: i for i, c in enumerate(_ALPHABET)}


def b58decode(s: str) -> bytes:
    n = 0
    for ch in s:
        try:
            n = n * 58 + _INDEX[ch]
        except KeyError:
            raise ValueError(f"invalid base58 character {ch!r}")
    pad = len(s) - len(s.lstrip("1"))
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    return b"\x00" * pad + body


def b58encode(b: bytes) -> str:
    n = int.from_bytes(b, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = _ALPHABET[r] + out
    pad = len(b) - len(b.lstrip(b"\x00"))
    return "1" * pad + out


if __name__ == "__main__":
    # CLI: decode a base58 argument to hex, for the shell case scripts.
    import sys

    print(b58decode(sys.argv[1]).hex())

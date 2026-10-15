"""`str.encode(encoding)` / `bytes.decode(encoding)` — mirrors src/interpreter/encoding.rs (ffi_value_exchange_plan.md 7-10).

Names are spelled as in Python (case and `-` / `_` are ignored):
  - utf-8, utf-16-le, utf-16-be, ascii, latin-1 with Python's meaning
  - shift_jis / cp932 / sjis / windows-31j / ms-kanji: the Windows CP932 table (Rust uses encoding_rs' Shift_JIS, the same table)
  - bare utf-16 is refused (no byte order mark is added or read)
Failing conversions raise ValueError with the same text as Rust; nothing is replaced with U+FFFD or `&#...;`.
"""
from __future__ import annotations


class EncodingValueError(Exception):
    """A conversion that fails: the caller raises it as the Arrow built-in ValueError with this message
    (Rust returns `Err("ValueError: ...")`, which becomes a catchable ValueError)."""

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message


_ALIASES = {
    "utf-8": "utf-8", "utf8": "utf-8", "u8": "utf-8",
    "utf-16-le": "utf-16-le", "utf-16le": "utf-16-le", "utf16le": "utf-16-le",
    "utf-16-be": "utf-16-be", "utf-16be": "utf-16-be", "utf16be": "utf-16-be",
    "ascii": "ascii", "us-ascii": "ascii",
    "latin-1": "latin-1", "latin1": "latin-1", "iso-8859-1": "latin-1", "iso8859-1": "latin-1", "l1": "latin-1",
    "shift-jis": "cp932", "sjis": "cp932", "cp932": "cp932", "ms932": "cp932", "windows-31j": "cp932", "ms-kanji": "cp932",
}


def _codec(name: str) -> str:
    norm = name.strip().lower().replace("_", "-")
    if norm in ("utf-16", "utf16"):
        raise EncodingValueError(
            f"encoding '{name}' is ambiguous: write 'utf-16-le' or 'utf-16-be' "
            "(no byte order mark is added or read)"
        )
    codec = _ALIASES.get(norm)
    if codec is None:
        raise EncodingValueError(f"unknown encoding '{name}'")
    return codec


def encode(s: str, encoding: str) -> bytes:
    codec = _codec(encoding)
    try:
        return s.encode(codec)
    except UnicodeEncodeError as e:
        c = s[e.start]
        raise EncodingValueError(
            f"'{encoding}' cannot encode character {_rust_char_debug(c)} (U+{ord(c):04X}) at position {e.start}"
        )


def decode(b: bytes, encoding: str) -> str:
    codec = _codec(encoding)
    if codec in ("utf-16-le", "utf-16-be") and len(b) % 2 != 0:
        raise EncodingValueError(f"cannot decode bytes as '{encoding}': odd length ({len(b)} bytes)")
    try:
        return b.decode(codec)
    except UnicodeDecodeError as e:
        if codec == "cp932":
            raise EncodingValueError(f"cannot decode bytes as '{encoding}': invalid byte sequence")
        raise EncodingValueError(f"cannot decode bytes as '{encoding}': invalid data at position {e.start}")


def _rust_char_debug(c: str) -> str:
    """Rust's `{c:?}` for a char: single quotes, with the escapes Rust uses for quotes and backslash."""
    if c == "'":
        return "'\\''"
    if c == "\\":
        return "'\\\\'"
    return f"'{c}'"

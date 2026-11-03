# binascii.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/binascii.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from typing import TypeAlias
from typing_extensions import deprecated

def a2b_uu(data: str | ReadableBuffer, /) -> bytes:
    ...

def b2a_uu(data: ReadableBuffer, /, *, backtick: bool=False) -> bytes:
    ...

def a2b_base64(data: str | ReadableBuffer, /, *, strict_mode: bool=False) -> bytes:
    ...

def b2a_base64(data: ReadableBuffer, /, *, newline: bool=True) -> bytes:
    ...

def a2b_qp(data: str | ReadableBuffer, header: bool=False) -> bytes:
    ...

def b2a_qp(data: ReadableBuffer, quotetabs: bool=False, istext: bool=True, header: bool=False) -> bytes:
    ...

def crc_hqx(data: ReadableBuffer, crc: int, /) -> int:
    ...

def crc32(data: ReadableBuffer, crc: int=0, /) -> int:
    ...

def b2a_hex(data: ReadableBuffer, sep: str | bytes=..., bytes_per_sep: int=1) -> bytes:
    ...

def hexlify(data: ReadableBuffer, sep: str | bytes=..., bytes_per_sep: int=1) -> bytes:
    ...

def a2b_hex(hexstr: str | ReadableBuffer, /) -> bytes:
    ...

def unhexlify(hexstr: str | ReadableBuffer, /) -> bytes:
    ...

class Error(ValueError):
    ...

class Incomplete(Exception):
    ...

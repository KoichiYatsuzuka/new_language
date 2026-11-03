# _struct.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_struct.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from _typeshed import ReadableBuffer, WriteableBuffer
from collections.abc import Iterator
from typing import Any
from typing_extensions import disjoint_base

def pack(fmt: str | bytes, /, *v: Any) -> bytes:
    ...

def pack_into(fmt: str | bytes, buffer: WriteableBuffer, offset: int, /, *v: Any) -> None:
    ...

def unpack(format: str | bytes, buffer: ReadableBuffer, /) -> tuple[Any, ...]:
    ...

def unpack_from(format: str | bytes, /, buffer: ReadableBuffer, offset: int=0) -> tuple[Any, ...]:
    ...

def iter_unpack(format: str | bytes, buffer: ReadableBuffer, /) -> Iterator[tuple[Any, ...]]:
    ...

def calcsize(format: str | bytes, /) -> int:
    ...

@disjoint_base
class Struct:

    @property
    def format(self) -> str:
        ...

    @property
    def size(self) -> int:
        ...

    def __init__(self, format: str | bytes) -> None:
        ...

    def pack(self, *v: Any) -> bytes:
        ...

    def pack_into(self, buffer: WriteableBuffer, offset: int, *v: Any) -> None:
        ...

    def unpack(self, buffer: ReadableBuffer, /) -> tuple[Any, ...]:
        ...

    def unpack_from(self, buffer: ReadableBuffer, offset: int=0) -> tuple[Any, ...]:
        ...

    def iter_unpack(self, buffer: ReadableBuffer, /) -> Iterator[tuple[Any, ...]]:
        ...

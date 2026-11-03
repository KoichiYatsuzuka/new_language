# _bz2.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_bz2.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from typing import final
from typing_extensions import Self

@final
class BZ2Compressor:

    def __init__(self, compresslevel: int=9, /) -> None:
        ...

    def compress(self, data: ReadableBuffer, /) -> bytes:
        ...

    def flush(self) -> bytes:
        ...

@final
class BZ2Decompressor:

    def decompress(self, data: ReadableBuffer, max_length: int=-1) -> bytes:
        ...

    @property
    def eof(self) -> bool:
        ...

    @property
    def needs_input(self) -> bool:
        ...

    @property
    def unused_data(self) -> bytes:
        ...

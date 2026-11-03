# _multibytecodec.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_multibytecodec.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from _typeshed import ReadableBuffer
from codecs import _ReadableStream, _WritableStream
from collections.abc import Iterable
from typing import final, type_check_only
from typing_extensions import disjoint_base

@final
@type_check_only
class _MultibyteCodec:

    def decode(self, input: ReadableBuffer, errors: str | None=None) -> str:
        ...

    def encode(self, input: str, errors: str | None=None) -> bytes:
        ...

@disjoint_base
class MultibyteIncrementalDecoder:
    errors: str

    def __init__(self, errors: str='strict') -> None:
        ...

    def decode(self, input: ReadableBuffer, final: bool=False) -> str:
        ...

    def getstate(self) -> tuple[bytes, int]:
        ...

    def reset(self) -> None:
        ...

    def setstate(self, state: tuple[bytes, int], /) -> None:
        ...

@disjoint_base
class MultibyteIncrementalEncoder:
    errors: str

    def __init__(self, errors: str='strict') -> None:
        ...

    def encode(self, input: str, final: bool=False) -> bytes:
        ...

    def getstate(self) -> int:
        ...

    def reset(self) -> None:
        ...

    def setstate(self, state: int, /) -> None:
        ...

@disjoint_base
class MultibyteStreamReader:
    errors: str
    stream: _ReadableStream

    def __init__(self, stream: _ReadableStream, errors: str='strict') -> None:
        ...

    def read(self, sizeobj: int | None=None, /) -> str:
        ...

    def readline(self, sizeobj: int | None=None, /) -> str:
        ...

    def readlines(self, sizehintobj: int | None=None, /) -> list[str]:
        ...

    def reset(self) -> None:
        ...

@disjoint_base
class MultibyteStreamWriter:
    errors: str
    stream: _WritableStream

    def __init__(self, stream: _WritableStream, errors: str='strict') -> None:
        ...

    def reset(self) -> None:
        ...

    def write(self, strobj: str, /) -> None:
        ...

    def writelines(self, lines: Iterable[str], /) -> None:
        ...

# unicodedata.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/unicodedata.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadOnlyBuffer
from collections.abc import Iterator
from typing import Final, Literal, TypeAlias, TypeVar, final, overload
ucd_3_2_0: UCD
unidata_version: Final[str]

def bidirectional(chr: str, /) -> str:
    ...

def category(chr: str, /) -> str:
    ...

def combining(chr: str, /) -> int:
    ...

@overload
def decimal(chr: str, /) -> int:
    ...

@overload
def decimal(chr: str, default: Any, /) -> int | Any:
    ...

def decomposition(chr: str, /) -> str:
    ...

@overload
def digit(chr: str, /) -> int:
    ...

@overload
def digit(chr: str, default: Any, /) -> int | Any:
    ...

def east_asian_width(chr: str, /) -> Literal['F', 'H', 'W', 'Na', 'A', 'N']:
    ...

def is_normalized(form: Literal['NFC', 'NFD', 'NFKC', 'NFKD'], unistr: str, /) -> bool:
    ...

def lookup(name: str | ReadOnlyBuffer, /) -> str:
    ...

def mirrored(chr: str, /) -> int:
    ...

@overload
def name(chr: str, /) -> str:
    ...

@overload
def name(chr: str, default: Any, /) -> str | Any:
    ...

def normalize(form: Literal['NFC', 'NFD', 'NFKC', 'NFKD'], unistr: str, /) -> str:
    ...

@overload
def numeric(chr: str, /) -> float:
    ...

@overload
def numeric(chr: str, default: Any, /) -> float | Any:
    ...

@final
class UCD:
    unidata_version: str

    def bidirectional(self, chr: str, /) -> str:
        ...

    def category(self, chr: str, /) -> str:
        ...

    def combining(self, chr: str, /) -> int:
        ...

    def decimal(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def decomposition(self, chr: str, /) -> str:
        ...

    def digit(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def east_asian_width(self, chr: str, /) -> Literal['F', 'H', 'W', 'Na', 'A', 'N']:
        ...

    def is_normalized(self, form: Literal['NFC', 'NFD', 'NFKC', 'NFKD'], unistr: str, /) -> bool:
        ...

    def lookup(self, name: str | ReadOnlyBuffer, /) -> str:
        ...

    def mirrored(self, chr: str, /) -> int:
        ...

    def name(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def normalize(self, form: Literal['NFC', 'NFD', 'NFKC', 'NFKD'], unistr: str, /) -> str:
        ...

    def numeric(self, *args: Any, **kwargs: Any) -> Any:
        ...

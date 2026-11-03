# _csv.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_csv.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import csv
import sys
from _typeshed import SupportsWrite
from collections.abc import Iterable
from typing import Any, Final, Literal, TypeAlias
from typing_extensions import Self, disjoint_base
__version__: Final[str]
QUOTE_ALL: int = 1
QUOTE_MINIMAL: int = 0
QUOTE_NONE: int = 3
QUOTE_NONNUMERIC: int = 2
QUOTE_STRINGS: int = 4
QUOTE_NOTNULL: int = 5

class Error(Exception):
    ...

@disjoint_base
class Dialect:
    delimiter: str
    quotechar: str | None
    escapechar: str | None
    doublequote: bool
    skipinitialspace: bool
    lineterminator: str
    quoting: Literal[0, 1, 2, 3, 4, 5]
    strict: bool

    def __init__(self, dialect: str | Dialect | csv.Dialect | type[Dialect | csv.Dialect] | None=None, delimiter: str=',', doublequote: bool=True, escapechar: str | None=None, lineterminator: str='\r\n', quotechar: str | None='"', quoting: Literal[0, 1, 2, 3, 4, 5]=0, skipinitialspace: bool=False, strict: bool=False) -> None:
        ...

@disjoint_base
class Reader:

    @property
    def dialect(self) -> Dialect:
        ...
    line_num: int

    def __iter__(self) -> Self:
        ...

    def __next__(self) -> list[str]:
        ...

@disjoint_base
class Writer:

    @property
    def dialect(self) -> Dialect:
        ...

    def writerow(self, row: Iterable[Any]) -> Any:
        ...

    def writerows(self, rows: Iterable[Iterable[Any]]) -> None:
        ...

def writer(fileobj: SupportsWrite[str], /, dialect: str | Dialect | csv.Dialect | type[Dialect | csv.Dialect]='excel', *, delimiter: str=',', quotechar: str | None='"', escapechar: str | None=None, doublequote: bool=True, skipinitialspace: bool=False, lineterminator: str='\r\n', quoting: Literal[0, 1, 2, 3, 4, 5]=0, strict: bool=False) -> Writer:
    ...

def reader(iterable: Iterable[str], /, dialect: str | Dialect | csv.Dialect | type[Dialect | csv.Dialect]='excel', *, delimiter: str=',', quotechar: str | None='"', escapechar: str | None=None, doublequote: bool=True, skipinitialspace: bool=False, lineterminator: str='\r\n', quoting: Literal[0, 1, 2, 3, 4, 5]=0, strict: bool=False) -> Reader:
    ...

def register_dialect(name: str, /, dialect: type[Dialect | csv.Dialect] | str='excel', *, delimiter: str=',', quotechar: str | None='"', escapechar: str | None=None, doublequote: bool=True, skipinitialspace: bool=False, lineterminator: str='\r\n', quoting: Literal[0, 1, 2, 3, 4, 5]=0, strict: bool=False) -> None:
    ...

def unregister_dialect(name: str) -> None:
    ...

def get_dialect(name: str) -> Dialect:
    ...

def list_dialects() -> list[str]:
    ...

def field_size_limit(new_limit: int=...) -> int:
    ...

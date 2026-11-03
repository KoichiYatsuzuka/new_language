# _heapq.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_heapq.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import SupportsRichComparisonT as _T
from typing import Final
__about__: Final[str]

def heapify(heap: list[_T], /) -> None:
    ...

def heappop(heap: list[_T], /) -> _T:
    ...

def heappush(heap: list[_T], item: _T, /) -> None:
    ...

def heappushpop(heap: list[_T], item: _T, /) -> _T:
    ...

def heapreplace(heap: list[_T], item: _T, /) -> _T:
    ...

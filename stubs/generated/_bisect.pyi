# _bisect.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_bisect.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from _typeshed import SupportsGetItem, SupportsLenAndGetItem, SupportsRichComparisonT
from collections.abc import Callable, MutableSequence
from typing import TypeVar, overload

@overload
def bisect_left(a: SupportsLenAndGetItem[SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: None=None) -> int:
    ...

@overload
def bisect_left(a: SupportsGetItem[int, SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int, hi: int, *, key: None=None) -> int:
    ...

@overload
def bisect_left(a: SupportsGetItem[int, SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, *, hi: int, key: None=None) -> int:
    ...

@overload
def bisect_left(a: SupportsLenAndGetItem[Any], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def bisect_left(a: SupportsGetItem[int, Any], x: SupportsRichComparisonT, lo: int, hi: int, *, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def bisect_left(a: SupportsGetItem[int, Any], x: SupportsRichComparisonT, lo: int=0, *, hi: int, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def bisect_right(a: SupportsLenAndGetItem[SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: None=None) -> int:
    ...

@overload
def bisect_right(a: SupportsGetItem[int, SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int, hi: int, *, key: None=None) -> int:
    ...

@overload
def bisect_right(a: SupportsGetItem[int, SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, *, hi: int, key: None=None) -> int:
    ...

@overload
def bisect_right(a: SupportsLenAndGetItem[Any], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def bisect_right(a: SupportsGetItem[int, Any], x: SupportsRichComparisonT, lo: int, hi: int, *, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def bisect_right(a: SupportsGetItem[int, Any], x: SupportsRichComparisonT, lo: int=0, *, hi: int, key: Callable[[Any], SupportsRichComparisonT]) -> int:
    ...

@overload
def insort_left(a: MutableSequence[SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: None=None) -> None:
    ...

@overload
def insort_left(a: MutableSequence[Any], x: Any, lo: int=0, hi: int | None=None, *, key: Callable[[Any], SupportsRichComparisonT]) -> None:
    ...

@overload
def insort_right(a: MutableSequence[SupportsRichComparisonT], x: SupportsRichComparisonT, lo: int=0, hi: int | None=None, *, key: None=None) -> None:
    ...

@overload
def insort_right(a: MutableSequence[Any], x: Any, lo: int=0, hi: int | None=None, *, key: Callable[[Any], SupportsRichComparisonT]) -> None:
    ...

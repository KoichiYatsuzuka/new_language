# _locale.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_locale.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import StrPath
from typing import Final, Literal, TypedDict, type_check_only

@type_check_only
class _LocaleConv(TypedDict):
    decimal_point: str
    grouping: list[int]
    thousands_sep: str
    int_curr_symbol: str
    currency_symbol: str
    p_cs_precedes: Literal[0, 1, 127]
    n_cs_precedes: Literal[0, 1, 127]
    p_sep_by_space: Literal[0, 1, 127]
    n_sep_by_space: Literal[0, 1, 127]
    mon_decimal_point: str
    frac_digits: int
    int_frac_digits: int
    mon_thousands_sep: str
    mon_grouping: list[int]
    positive_sign: str
    negative_sign: str
    p_sign_posn: Literal[0, 1, 2, 3, 4, 127]
    n_sign_posn: Literal[0, 1, 2, 3, 4, 127]
LC_CTYPE: Final[int]
LC_COLLATE: Final[int]
LC_TIME: Final[int]
LC_MONETARY: Final[int]
LC_NUMERIC: Final[int]
LC_ALL: Final[int]
CHAR_MAX: int = 127

def setlocale(category: int, locale: str | None=None, /) -> str:
    ...

def localeconv() -> _LocaleConv:
    ...

def getencoding() -> str:
    ...

def strcoll(os1: str, os2: str, /) -> int:
    ...

def strxfrm(string: str, /) -> str:
    ...

# _decimal.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_decimal.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from decimal import Clamped as Clamped, Context as Context, ConversionSyntax as ConversionSyntax, Decimal as Decimal, DecimalException as DecimalException, DecimalTuple as DecimalTuple, DivisionByZero as DivisionByZero, DivisionImpossible as DivisionImpossible, DivisionUndefined as DivisionUndefined, FloatOperation as FloatOperation, Inexact as Inexact, InvalidContext as InvalidContext, InvalidOperation as InvalidOperation, Overflow as Overflow, Rounded as Rounded, Subnormal as Subnormal, Underflow as Underflow, _ContextManager
from typing import Final, TypeAlias
__version__: Final[str]
__libmpdec_version__: Final[str]
ROUND_DOWN: str = 'ROUND_DOWN'
ROUND_HALF_UP: str = 'ROUND_HALF_UP'
ROUND_HALF_EVEN: str = 'ROUND_HALF_EVEN'
ROUND_CEILING: str = 'ROUND_CEILING'
ROUND_FLOOR: str = 'ROUND_FLOOR'
ROUND_UP: str = 'ROUND_UP'
ROUND_HALF_DOWN: str = 'ROUND_HALF_DOWN'
ROUND_05UP: str = 'ROUND_05UP'
HAVE_CONTEXTVAR: Final[bool]
HAVE_THREADS: Final[bool]
MAX_EMAX: Final[int]
MAX_PREC: Final[int]
MIN_EMIN: Final[int]
MIN_ETINY: Final[int]

def setcontext(context: Context, /) -> None:
    ...

def getcontext() -> Context:
    ...

def localcontext(ctx: Context | None=None, *, prec: int | None=None, rounding: str | None=None, Emin: int | None=None, Emax: int | None=None, capitals: int | None=None, clamp: int | None=None, traps: dict[type[DecimalException], bool] | None=None, flags: dict[type[DecimalException], bool] | None=None) -> _ContextManager:
    ...
DefaultContext: Context
BasicContext: Context
ExtendedContext: Context

# _operator.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_operator.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import SupportsAdd, SupportsGetItem, SupportsMod, SupportsMul, SupportsRAdd, SupportsRMod, SupportsRMul, SupportsRSub, SupportsSub
from collections.abc import Callable, Container, Iterable, MutableMapping, MutableSequence, Sequence
from operator import attrgetter as attrgetter, itemgetter as itemgetter, methodcaller as methodcaller
from typing import Any, AnyStr, ParamSpec, Protocol, SupportsAbs, SupportsIndex, TypeAlias, TypeVar, overload, type_check_only
from typing_extensions import TypeIs

@type_check_only
class _SupportsDunderLT(Protocol):

    def __lt__(self, other: Any, /) -> Any:
        ...

@type_check_only
class _SupportsDunderGT(Protocol):

    def __gt__(self, other: Any, /) -> Any:
        ...

@type_check_only
class _SupportsDunderLE(Protocol):

    def __le__(self, other: Any, /) -> Any:
        ...

@type_check_only
class _SupportsDunderGE(Protocol):

    def __ge__(self, other: Any, /) -> Any:
        ...

@type_check_only
class _SupportsInversion(Protocol[_T_co]):

    def __invert__(self) -> Any:
        ...

@type_check_only
class _SupportsNeg(Protocol[_T_co]):

    def __neg__(self) -> Any:
        ...

@type_check_only
class _SupportsPos(Protocol[_T_co]):

    def __pos__(self) -> Any:
        ...

def lt(a: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, b: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, /) -> Any:
    ...

def le(a: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, b: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, /) -> Any:
    ...

def eq(a: object, b: object, /) -> Any:
    ...

def ne(a: object, b: object, /) -> Any:
    ...

def ge(a: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, b: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, /) -> Any:
    ...

def gt(a: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, b: _SupportsDunderLE | _SupportsDunderGE | _SupportsDunderGT | _SupportsDunderLT, /) -> Any:
    ...

def not_(a: object, /) -> bool:
    ...

def truth(a: object, /) -> bool:
    ...

def is_(a: object, b: object, /) -> bool:
    ...

def is_not(a: object, b: object, /) -> bool:
    ...

def abs(a: SupportsAbs[Any], /) -> Any:
    ...

@overload
def add(a: SupportsAdd[Any, Any], b: Any, /) -> Any:
    ...

@overload
def add(a: Any, b: SupportsRAdd[Any, Any], /) -> Any:
    ...

def and_(a, b, /):
    ...

def floordiv(a, b, /):
    ...

def index(a: SupportsIndex, /) -> int:
    ...

def inv(a: _SupportsInversion[Any], /) -> Any:
    ...

def invert(a: _SupportsInversion[Any], /) -> Any:
    ...

def lshift(a, b, /):
    ...

@overload
def mod(a: SupportsMod[Any, Any], b: Any, /) -> Any:
    ...

@overload
def mod(a: Any, b: SupportsRMod[Any, Any], /) -> Any:
    ...

@overload
def mul(a: SupportsMul[Any, Any], b: Any, /) -> Any:
    ...

@overload
def mul(a: Any, b: SupportsRMul[Any, Any], /) -> Any:
    ...

def matmul(a, b, /):
    ...

def neg(a: _SupportsNeg[Any], /) -> Any:
    ...

def or_(a, b, /):
    ...

def pos(a: _SupportsPos[Any], /) -> Any:
    ...

def pow(a, b, /):
    ...

def rshift(a, b, /):
    ...

@overload
def sub(a: SupportsSub[Any, Any], b: Any, /) -> Any:
    ...

@overload
def sub(a: Any, b: SupportsRSub[Any, Any], /) -> Any:
    ...

def truediv(a, b, /):
    ...

def xor(a, b, /):
    ...

def concat(a: Sequence[Any], b: Sequence[Any], /) -> Sequence[Any]:
    ...

def contains(a: Container[object], b: object, /) -> bool:
    ...

def countOf(a: Iterable[object], b: object, /) -> int:
    ...

@overload
def delitem(a: MutableSequence[Any], b: int, /) -> None:
    ...

@overload
def delitem(a: MutableSequence[Any], b: slice[int | None], /) -> None:
    ...

@overload
def delitem(a: MutableMapping[Any, Any], b: Any, /) -> None:
    ...

@overload
def getitem(a: Sequence[Any], b: slice[int | None], /) -> Sequence[Any]:
    ...

@overload
def getitem(a: SupportsGetItem[Any, Any], b: Any, /) -> Any:
    ...

def indexOf(a: Iterable[Any], b: Any, /) -> int:
    ...

@overload
def setitem(a: MutableSequence[Any], b: int, c: Any, /) -> None:
    ...

@overload
def setitem(a: MutableSequence[Any], b: slice[int | None], c: Sequence[Any], /) -> None:
    ...

@overload
def setitem(a: MutableMapping[Any, Any], b: Any, c: Any, /) -> None:
    ...

def length_hint(obj: object, default: int=0, /) -> int:
    ...

def iadd(a, b, /):
    ...

def iand(a, b, /):
    ...

def iconcat(a, b, /):
    ...

def ifloordiv(a, b, /):
    ...

def ilshift(a, b, /):
    ...

def imod(a, b, /):
    ...

def imul(a, b, /):
    ...

def imatmul(a, b, /):
    ...

def ior(a, b, /):
    ...

def ipow(a, b, /):
    ...

def irshift(a, b, /):
    ...

def isub(a, b, /):
    ...

def itruediv(a, b, /):
    ...

def ixor(a, b, /):
    ...

def call(obj: Callable[Any, Any], /, *args: Any.args, **kwargs: Any.kwargs) -> Any:
    ...

def _compare_digest(a: AnyStr, b: AnyStr, /) -> bool:
    ...

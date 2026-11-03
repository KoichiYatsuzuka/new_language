# builtins.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/builtins.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import _ast
import _sitebuiltins
import _typeshed
import sys
import types
from _collections_abc import dict_items, dict_keys, dict_values
from _typeshed import AnnotationForm, ConvertibleToFloat, ConvertibleToInt, FileDescriptorOrPath, OpenBinaryMode, OpenBinaryModeReading, OpenBinaryModeUpdating, OpenBinaryModeWriting, OpenTextMode, ReadableBuffer, SupportsAdd, SupportsAiter, SupportsAnext, SupportsDivMod, SupportsFlush, SupportsIter, SupportsKeysAndGetItem, SupportsLenAndGetItem, SupportsNext, SupportsRAdd, SupportsRDivMod, SupportsRichComparison, SupportsRichComparisonT, SupportsWrite
from collections.abc import Awaitable, Callable, Iterable, Iterator, MutableSet, Reversible, Set as AbstractSet, Sized
from io import BufferedRandom, BufferedReader, BufferedWriter, FileIO, TextIOWrapper
from os import PathLike
from types import CellType, CodeType, EllipsisType, GenericAlias, NotImplementedType, TracebackType
from typing import IO, Any, BinaryIO, ClassVar, Concatenate, Final, Generic, Mapping, MutableMapping, MutableSequence, ParamSpec, Protocol, Sequence, SupportsAbs, SupportsBytes, SupportsComplex, SupportsFloat, SupportsIndex, TypeAlias, TypeGuard, TypeVar, final, overload, type_check_only
from typing_extensions import Literal, LiteralString, Self, TypeIs, TypeVarTuple, deprecated, disjoint_base

def abs(x: SupportsAbs[Any], /) -> Any:
    ...

def all(iterable: Iterable[object], /) -> bool:
    ...

def any(iterable: Iterable[object], /) -> bool:
    ...

def ascii(obj: object, /) -> str:
    ...

def bin(number: SupportsIndex, /) -> str:
    ...

def breakpoint(*args: Any, **kws: Any) -> None:
    ...

def callable(obj: object, /) -> TypeIs[Callable[..., object]]:
    ...

def chr(i: SupportsIndex, /) -> str:
    ...

def aiter(async_iterable: SupportsAiter[Any], /) -> Any:
    ...

@overload
def anext(i: _SupportsSynchronousAnext[Any], /) -> Any:
    ...

@overload
async def anext(i: SupportsAnext[Any], default: Any, /) -> Any | Any:
    ...

@overload
def compile(source: str | ReadableBuffer | _ast.Module | _ast.Expression | _ast.Interactive, filename: str | bytes | PathLike[Any], mode: str, flags: Literal[0], dont_inherit: bool=False, optimize: int=-1, *, _feature_version: int=-1) -> CodeType:
    ...

@overload
def compile(source: str | ReadableBuffer | _ast.Module | _ast.Expression | _ast.Interactive, filename: str | bytes | PathLike[Any], mode: str, *, dont_inherit: bool=False, optimize: int=-1, _feature_version: int=-1) -> CodeType:
    ...

@overload
def compile(source: str | ReadableBuffer | _ast.Module | _ast.Expression | _ast.Interactive, filename: str | bytes | PathLike[Any], mode: str, flags: Literal[1024], dont_inherit: bool=False, optimize: int=-1, *, _feature_version: int=-1) -> _ast.AST:
    ...

@overload
def compile(source: str | ReadableBuffer | _ast.Module | _ast.Expression | _ast.Interactive, filename: str | bytes | PathLike[Any], mode: str, flags: int, dont_inherit: bool=False, optimize: int=-1, *, _feature_version: int=-1) -> Any:
    ...
copyright: _sitebuiltins._Printer
credits: _sitebuiltins._Printer

def delattr(obj: object, name: str, /) -> None:
    ...

def dir(o: object=..., /) -> list[str]:
    ...

@overload
def divmod(x: SupportsDivMod[Any, Any], y: Any, /) -> Any:
    ...

@overload
def divmod(x: Any, y: SupportsRDivMod[Any, Any], /) -> Any:
    ...

def eval(source: str | ReadableBuffer | CodeType, globals: dict[str, Any] | None=None, locals: Mapping[str, object] | None=None, /) -> Any:
    ...

def exec(source: str | ReadableBuffer | CodeType, globals: dict[str, Any] | None=None, locals: Mapping[str, object] | None=None, /, *, closure: tuple[CellType, ...] | None=None) -> None:
    ...
exit: _sitebuiltins.Quitter

def format(value: object, format_spec: str='', /) -> str:
    ...

@overload
def getattr(o: object, name: str, /) -> Any:
    ...

@overload
def getattr(o: object, name: str, default: None, /) -> Any | None:
    ...

@overload
def getattr(o: object, name: str, default: bool, /) -> Any | bool:
    ...

@overload
def getattr(o: object, name: str, default: list[Any], /) -> Any | list[Any]:
    ...

@overload
def getattr(o: object, name: str, default: dict[Any, Any], /) -> Any | dict[Any, Any]:
    ...

@overload
def getattr(o: object, name: str, default: Any, /) -> Any | Any:
    ...

def globals() -> dict[str, Any]:
    ...

def hasattr(obj: object, name: str, /) -> bool:
    ...

def hash(obj: object, /) -> int:
    ...
help: _sitebuiltins._Helper

def hex(number: SupportsIndex, /) -> str:
    ...

def id(obj: object, /) -> int:
    ...

def input(prompt: object='', /) -> str:
    ...

@overload
def iter(object: SupportsIter[Any], /) -> Any:
    ...

@overload
def iter(object: _GetItemIterable[Any], /) -> Iterator[Any]:
    ...

@overload
def iter(object: Callable[[], Any | None], sentinel: None, /) -> Iterator[Any]:
    ...

@overload
def iter(object: Callable[[], Any], sentinel: object, /) -> Iterator[Any]:
    ...

def isinstance(obj: object, class_or_tuple: type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[_ClassInfo, ...], ...], ...], ...], ...], ...], ...], ...], ...], ...], /) -> bool:
    ...

def issubclass(cls: type, class_or_tuple: type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[type | types.UnionType | tuple[_ClassInfo, ...], ...], ...], ...], ...], ...], ...], ...], ...], ...], /) -> bool:
    ...

def len(obj: Sized, /) -> int:
    ...
license: _sitebuiltins._Printer

def locals() -> dict[str, Any]:
    ...

@overload
def max(arg1: SupportsRichComparisonT, arg2: SupportsRichComparisonT, /, *_args: SupportsRichComparisonT, key: None=None) -> SupportsRichComparisonT:
    ...

@overload
def max(arg1: Any, arg2: Any, /, *_args: Any, key: Callable[[Any], SupportsRichComparison]) -> Any:
    ...

@overload
def max(iterable: Iterable[SupportsRichComparisonT], /, *, key: None=None) -> SupportsRichComparisonT:
    ...

@overload
def max(iterable: Iterable[Any], /, *, key: Callable[[Any], SupportsRichComparison]) -> Any:
    ...

@overload
def max(iterable: Iterable[SupportsRichComparisonT], /, *, key: None=None, default: Any) -> SupportsRichComparisonT | Any:
    ...

@overload
def max(iterable: Iterable[Any], /, *, key: Callable[[Any], SupportsRichComparison], default: Any) -> Any | Any:
    ...

@overload
def min(arg1: SupportsRichComparisonT, arg2: SupportsRichComparisonT, /, *_args: SupportsRichComparisonT, key: None=None) -> SupportsRichComparisonT:
    ...

@overload
def min(arg1: Any, arg2: Any, /, *_args: Any, key: Callable[[Any], SupportsRichComparison]) -> Any:
    ...

@overload
def min(iterable: Iterable[SupportsRichComparisonT], /, *, key: None=None) -> SupportsRichComparisonT:
    ...

@overload
def min(iterable: Iterable[Any], /, *, key: Callable[[Any], SupportsRichComparison]) -> Any:
    ...

@overload
def min(iterable: Iterable[SupportsRichComparisonT], /, *, key: None=None, default: Any) -> SupportsRichComparisonT | Any:
    ...

@overload
def min(iterable: Iterable[Any], /, *, key: Callable[[Any], SupportsRichComparison], default: Any) -> Any | Any:
    ...

@overload
def next(i: SupportsNext[Any], /) -> Any:
    ...

@overload
def next(i: SupportsNext[Any], default: Any, /) -> Any | Any:
    ...

def oct(number: SupportsIndex, /) -> str:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenTextMode='r', buffering: int=-1, encoding: str | None=None, errors: str | None=None, newline: str | None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> TextIOWrapper:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenBinaryMode, buffering: Literal[0], encoding: None=None, errors: None=None, newline: None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> FileIO:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenBinaryModeUpdating, buffering: Literal[-1, 1]=-1, encoding: None=None, errors: None=None, newline: None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> BufferedRandom:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenBinaryModeWriting, buffering: Literal[-1, 1]=-1, encoding: None=None, errors: None=None, newline: None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> BufferedWriter:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenBinaryModeReading, buffering: Literal[-1, 1]=-1, encoding: None=None, errors: None=None, newline: None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> BufferedReader:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: OpenBinaryMode, buffering: int=-1, encoding: None=None, errors: None=None, newline: None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> BinaryIO:
    ...

@overload
def open(file: FileDescriptorOrPath, mode: str, buffering: int=-1, encoding: str | None=None, errors: str | None=None, newline: str | None=None, closefd: bool=True, opener: Callable[[str, int], int] | None=None) -> IO[Any]:
    ...

def ord(c: str | bytes | bytearray, /) -> int:
    ...

@overload
def print(*values: object, sep: str | None=' ', end: str | None='\n', file: SupportsWrite[str] | None=None, flush: Literal[False]=False) -> None:
    ...

@overload
def print(*values: object, sep: str | None=' ', end: str | None='\n', file: _SupportsWriteAndFlush[str] | None=None, flush: bool) -> None:
    ...

@overload
def pow(base: int, exp: int, mod: int) -> int:
    ...

@overload
def pow(base: int, exp: Literal[0], mod: None=None) -> Literal[1]:
    ...

@overload
def pow(base: int, exp: Literal[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25], mod: None=None) -> int:
    ...

@overload
def pow(base: int, exp: Literal[-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16, -17, -18, -19, -20], mod: None=None) -> float:
    ...

@overload
def pow(base: int, exp: int, mod: None=None) -> Any:
    ...

@overload
def pow(base: Literal[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25], exp: float, mod: None=None) -> float:
    ...

@overload
def pow(base: Literal[-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16, -17, -18, -19, -20], exp: float, mod: None=None) -> complex:
    ...

@overload
def pow(base: float, exp: int, mod: None=None) -> float:
    ...

@overload
def pow(base: float, exp: complex | (_SupportsPow2[Any, Any] | _SupportsPow3NoneOnly[Any, Any] | _SupportsPow3[Any, Any, Any]), mod: None=None) -> Any:
    ...

@overload
def pow(base: complex, exp: complex | (_SupportsPow2[Any, Any] | _SupportsPow3NoneOnly[Any, Any] | _SupportsPow3[Any, Any, Any]), mod: None=None) -> complex:
    ...

@overload
def pow(base: _SupportsPow2[Any, Any], exp: Any, mod: None=None) -> Any:
    ...

@overload
def pow(base: _SupportsPow3NoneOnly[Any, Any], exp: Any, mod: None=None) -> Any:
    ...

@overload
def pow(base: _SupportsPow3[Any, Any, Any], exp: Any, mod: Any) -> Any:
    ...

@overload
def pow(base: _SupportsPow2[Any, Any] | _SupportsPow3NoneOnly[Any, Any] | _SupportsPow3[Any, Any, Any], exp: float, mod: None=None) -> Any:
    ...

@overload
def pow(base: _SupportsPow2[Any, Any] | _SupportsPow3NoneOnly[Any, Any] | _SupportsPow3[Any, Any, Any], exp: complex, mod: None=None) -> complex:
    ...
quit: _sitebuiltins.Quitter

def repr(obj: object, /) -> str:
    ...

@overload
def round(number: _SupportsRound1[Any], ndigits: None=None) -> Any:
    ...

@overload
def round(number: _SupportsRound2[Any], ndigits: SupportsIndex) -> Any:
    ...

def setattr(obj: object, name: str, value: Any, /) -> None:
    ...

@overload
def sorted(iterable: Iterable[SupportsRichComparisonT], /, *, key: None=None, reverse: bool=False) -> list[SupportsRichComparisonT]:
    ...

@overload
def sorted(iterable: Iterable[Any], /, *, key: Callable[[Any], SupportsRichComparison], reverse: bool=False) -> list[Any]:
    ...

@overload
def sum(iterable: Iterable[bool | (Literal[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25] | Literal[-1, -2, -3, -4, -5, -6, -7, -8, -9, -10, -11, -12, -13, -14, -15, -16, -17, -18, -19, -20] | Literal[0])], /, start: int=0) -> int:
    ...

@overload
def sum(iterable: Iterable[Any], /) -> Any | Literal[0]:
    ...

@overload
def sum(iterable: Iterable[Any], /, start: Any) -> Any | Any:
    ...

@overload
def vars(object: type, /) -> types.MappingProxyType[str, Any]:
    ...

@overload
def vars(object: Any=..., /) -> dict[str, Any]:
    ...

def __import__(name: str, globals: Mapping[str, object] | None=None, locals: Mapping[str, object] | None=None, fromlist: Sequence[str] | None=(), level: int=0) -> types.ModuleType:
    ...

def __build_class__(func: Callable[[], CellType | Any], name: str, /, *bases: Any, metaclass: Any=..., **kwds: Any) -> Any:
    ...
ellipsis = EllipsisType
Ellipsis: EllipsisType
NotImplemented: NotImplementedType
EnvironmentError = OSError
IOError = OSError
WindowsError = OSError

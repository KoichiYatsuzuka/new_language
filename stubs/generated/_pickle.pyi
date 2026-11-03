# _pickle.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_pickle.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from _typeshed import ReadableBuffer, SupportsWrite
from collections.abc import Callable, Iterable, Iterator, Mapping
from pickle import PickleBuffer as PickleBuffer
from typing import Any, Protocol, TypeAlias, type_check_only
from typing_extensions import disjoint_base

@type_check_only
class _ReadableFileobj(Protocol):

    def read(self, n: int, /) -> bytes:
        ...

    def readline(self) -> bytes:
        ...

def dump(obj: Any, file: SupportsWrite[bytes], protocol: int | None=None, *, fix_imports: bool=True, buffer_callback: Callable[[PickleBuffer], Any] | None=None) -> None:
    ...

def dumps(obj: Any, protocol: int | None=None, *, fix_imports: bool=True, buffer_callback: Callable[[PickleBuffer], Any] | None=None) -> bytes:
    ...

def load(file: _ReadableFileobj, *, fix_imports: bool=True, encoding: str='ASCII', errors: str='strict', buffers: Iterable[Any] | None=()) -> Any:
    ...

def loads(data: ReadableBuffer, /, *, fix_imports: bool=True, encoding: str='ASCII', errors: str='strict', buffers: Iterable[Any] | None=()) -> Any:
    ...

class PickleError(Exception):
    ...

class PicklingError(PickleError):
    ...

class UnpicklingError(PickleError):
    ...

@type_check_only
class PicklerMemoProxy:

    def clear(self, /) -> None:
        ...

    def copy(self, /) -> dict[int, tuple[int, Any]]:
        ...

@disjoint_base
class Pickler:
    fast: bool
    dispatch_table: Mapping[type, Callable[[Any], str | tuple[Callable[..., Any], tuple[Any, ...]] | tuple[Callable[..., Any], tuple[Any, ...], Any] | tuple[Callable[..., Any], tuple[Any, ...], Any, Iterator[Any] | None] | tuple[Callable[..., Any], tuple[Any, ...], Any, Iterator[Any] | None, Iterator[Any] | None]]]
    bin: bool

    def __init__(self, file: SupportsWrite[bytes], protocol: int | None=None, fix_imports: bool=True, buffer_callback: Callable[[PickleBuffer], Any] | None=None) -> None:
        ...

    @property
    def memo(self) -> PicklerMemoProxy:
        ...

    @memo.setter
    def memo(self, value: PicklerMemoProxy | dict[int, tuple[int, Any]]) -> None:
        ...

    def dump(self, obj: Any, /) -> None:
        ...

    def clear_memo(self) -> None:
        ...

    def persistent_id(self, obj: Any, /) -> Any:
        ...

    def reducer_override(self, obj: object, /) -> str | tuple[Callable[..., Any], tuple[Any, ...]] | tuple[Callable[..., Any], tuple[Any, ...], Any] | tuple[Callable[..., Any], tuple[Any, ...], Any, Iterator[Any] | None] | tuple[Callable[..., Any], tuple[Any, ...], Any, Iterator[Any] | None, Iterator[Any] | None]:
        ...

@type_check_only
class UnpicklerMemoProxy:

    def clear(self, /) -> None:
        ...

    def copy(self, /) -> dict[int, tuple[int, Any]]:
        ...

@disjoint_base
class Unpickler:

    def __init__(self, file: _ReadableFileobj, *, fix_imports: bool=True, encoding: str='ASCII', errors: str='strict', buffers: Iterable[Any] | None=()) -> None:
        ...

    @property
    def memo(self) -> UnpicklerMemoProxy:
        ...

    @memo.setter
    def memo(self, value: UnpicklerMemoProxy | dict[int, tuple[int, Any]]) -> None:
        ...

    def load(self) -> Any:
        ...

    def find_class(self, module_name: str, global_name: str, /) -> Any:
        ...

    def persistent_load(self, pid: Any, /) -> Any:
        ...

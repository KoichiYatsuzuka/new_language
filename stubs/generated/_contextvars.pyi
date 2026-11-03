# _contextvars.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_contextvars.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from collections.abc import Callable, Iterator, Mapping
from types import GenericAlias, TracebackType
from typing import Any, ClassVar, Generic, ParamSpec, TypeVar, final, overload
from typing_extensions import Self

@final
class ContextVar(Generic[_T]):

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        ...

    def __hash__(self) -> int:
        ...

    @property
    def name(self) -> str:
        ...

    def get(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def set(self, value: Any, /) -> Token[Any]:
        ...

    def reset(self, token: Token[Any], /) -> None:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

@final
class Token(Generic[_T]):

    @property
    def var(self) -> ContextVar[Any]:
        ...

    @property
    def old_value(self) -> Any:
        ...
    MISSING: ClassVar[object]
    __hash__: ClassVar[None]

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

def copy_context() -> Context:
    ...

@final
class Context(Mapping[ContextVar[Any], Any]):

    def __init__(self) -> None:
        ...

    def get(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def run(self, callable: Callable[Any, Any], *args: Any.args, **kwargs: Any.kwargs) -> Any:
        ...

    def copy(self) -> Context:
        ...
    __hash__: ClassVar[None]

    def __getitem__(self, key: ContextVar[Any], /) -> Any:
        ...

    def __iter__(self) -> Iterator[ContextVar[Any]]:
        ...

    def __len__(self) -> int:
        ...

    def __eq__(self, value: object, /) -> bool:
        ...

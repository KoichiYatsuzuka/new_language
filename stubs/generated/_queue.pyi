# _queue.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_queue.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from types import GenericAlias
from typing import Any, Generic, TypeVar
from typing_extensions import disjoint_base

class Empty(Exception):
    ...

@disjoint_base
class SimpleQueue(Generic[_T]):

    def __init__(self) -> None:
        ...

    def empty(self) -> bool:
        ...

    def get(self, block: bool=True, timeout: float | None=None) -> Any:
        ...

    def get_nowait(self) -> Any:
        ...

    def put(self, item: Any, block: bool=True, timeout: float | None=None) -> None:
        ...

    def put_nowait(self, item: Any) -> None:
        ...

    def qsize(self) -> int:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

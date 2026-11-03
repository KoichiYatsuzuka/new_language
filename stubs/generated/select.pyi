# select.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/select.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import FileDescriptorLike
from collections.abc import Iterable
from types import TracebackType
from typing import Any, ClassVar, Final, TypeVar, final, overload
from typing_extensions import Never, Self, deprecated

def select(rlist: Iterable[Any], wlist: Iterable[Any], xlist: Iterable[Any], timeout: float | None=None, /) -> tuple[list[Any], list[Any], list[Any]]:
    ...
error = OSError

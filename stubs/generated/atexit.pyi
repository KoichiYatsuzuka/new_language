# atexit.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/atexit.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from collections.abc import Callable
from typing import ParamSpec, TypeVar

def _clear() -> None:
    ...

def _ncallbacks() -> int:
    ...

def _run_exitfuncs() -> None:
    ...

def register(func: Callable[Any, Any], /, *args: Any.args, **kwargs: Any.kwargs) -> Callable[Any, Any]:
    ...

def unregister(func: Callable[..., object], /) -> None:
    ...

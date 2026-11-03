# _warnings.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_warnings.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from typing import Any, overload
_defaultaction: str
_onceregistry: dict[Any, Any]
filters: list[tuple[str, str | None, type[Warning], str | None, int]]

@overload
def warn(message: str, category: type[Warning] | None=None, stacklevel: int=1, source: Any | None=None, *, skip_file_prefixes: tuple[str, ...]=()) -> None:
    ...

@overload
def warn(message: Warning, category: Any=None, stacklevel: int=1, source: Any | None=None, *, skip_file_prefixes: tuple[str, ...]=()) -> None:
    ...

@overload
def warn_explicit(message: str, category: type[Warning], filename: str, lineno: int, module: str | None=..., registry: dict[str | tuple[str, type[Warning], int], int] | None=None, module_globals: dict[str, Any] | None=None, source: Any | None=None) -> None:
    ...

@overload
def warn_explicit(message: Warning, category: Any, filename: str, lineno: int, module: str | None=None, registry: dict[str | tuple[str, type[Warning], int], int] | None=None, module_globals: dict[str, Any] | None=None, source: Any | None=None) -> None:
    ...

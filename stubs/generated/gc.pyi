# gc.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/gc.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from collections.abc import Callable
from typing import Any, Final, Literal, TypeAlias
DEBUG_COLLECTABLE: int = 2
DEBUG_LEAK: int = 38
DEBUG_SAVEALL: int = 32
DEBUG_STATS: int = 1
DEBUG_UNCOLLECTABLE: int = 4
callbacks: list[Callable[[Literal['start', 'stop'], dict[str, int]], object]]
garbage: list[Any]

def collect(generation: int=2) -> int:
    ...

def disable() -> None:
    ...

def enable() -> None:
    ...

def get_count() -> tuple[int, int, int]:
    ...

def get_debug() -> int:
    ...

def get_objects(generation: int | None=None) -> list[Any]:
    ...

def freeze() -> None:
    ...

def unfreeze() -> None:
    ...

def get_freeze_count() -> int:
    ...

def get_referents(*objs: Any) -> list[Any]:
    ...

def get_referrers(*objs: Any) -> list[Any]:
    ...

def get_stats() -> list[dict[str, Any]]:
    ...

def get_threshold() -> tuple[int, int, int]:
    ...

def is_tracked(obj: Any, /) -> bool:
    ...

def is_finalized(obj: Any, /) -> bool:
    ...

def isenabled() -> bool:
    ...

def set_debug(flags: int, /) -> None:
    ...

def set_threshold(threshold0: int, threshold1: int=0, threshold2: int=0, /) -> None:
    ...

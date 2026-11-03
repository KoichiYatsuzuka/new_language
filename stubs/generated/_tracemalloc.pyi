# _tracemalloc.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_tracemalloc.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from collections.abc import Sequence
from tracemalloc import _FrameTuple, _TraceTuple

def _get_object_traceback(obj: object, /) -> Sequence[_FrameTuple] | None:
    ...

def _get_traces() -> Sequence[_TraceTuple]:
    ...

def clear_traces() -> None:
    ...

def get_traceback_limit() -> int:
    ...

def get_traced_memory() -> tuple[int, int]:
    ...

def get_tracemalloc_memory() -> int:
    ...

def is_tracing() -> bool:
    ...

def reset_peak() -> None:
    ...

def start(nframe: int=1, /) -> None:
    ...

def stop() -> None:
    ...

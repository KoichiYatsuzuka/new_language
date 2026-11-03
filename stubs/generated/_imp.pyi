# _imp.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_imp.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
import types
from _typeshed import ReadableBuffer
from importlib.machinery import ModuleSpec
from typing import Any
check_hash_based_pycs: str

def source_hash(key: int, source: ReadableBuffer) -> bytes:
    ...

def create_builtin(spec: ModuleSpec, /) -> types.ModuleType:
    ...

def create_dynamic(spec: ModuleSpec, file: Any=None, /) -> types.ModuleType:
    ...

def acquire_lock() -> None:
    ...

def exec_builtin(mod: types.ModuleType, /) -> int:
    ...

def exec_dynamic(mod: types.ModuleType, /) -> int:
    ...

def extension_suffixes() -> list[str]:
    ...

def init_frozen(name: str, /) -> types.ModuleType:
    ...

def is_builtin(name: str, /) -> int:
    ...

def is_frozen(name: str, /) -> bool:
    ...

def is_frozen_package(name: str, /) -> bool:
    ...

def lock_held() -> bool:
    ...

def release_lock() -> None:
    ...

def find_frozen(name: str, /, *, withdata: bool=False) -> tuple[memoryview | None, bool, str | None] | None:
    ...

def get_frozen_object(name: str, data: ReadableBuffer | None=None, /) -> types.CodeType:
    ...

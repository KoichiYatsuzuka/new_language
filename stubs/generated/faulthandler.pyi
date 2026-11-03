# faulthandler.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/faulthandler.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import FileDescriptorLike

def cancel_dump_traceback_later() -> None:
    ...

def disable() -> None:
    ...

def dump_traceback(file: FileDescriptorLike=sys.stderr, all_threads: bool=True) -> None:
    ...

def dump_traceback_later(timeout: float, repeat: bool=False, file: FileDescriptorLike=sys.stderr, exit: bool=False) -> None:
    ...

def enable(file: FileDescriptorLike=sys.stderr, all_threads: bool=True) -> None:
    ...

def is_enabled() -> bool:
    ...

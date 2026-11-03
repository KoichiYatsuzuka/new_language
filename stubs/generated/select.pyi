# select.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

class error:
    def __init__(self, *args: Any, **kwargs: Any) -> None: ...
    def __str__(self) -> Any: ...
    characters_written: Any
    errno: Any
    filename: Any
    filename2: Any
    strerror: Any
    winerror: Any
    __cause__: Any
    __context__: Any
    def __repr__(self) -> Any: ...
    __suppress_context__: Any
    __traceback__: Any
    def add_note(self, *args: Any, **kwargs: Any) -> Any: ...
    args: Any
    def with_traceback(self, *args: Any, **kwargs: Any) -> Any: ...
def select(rlist: Any, wlist: Any, xlist: Any, timeout: Any = ..., /) -> Any: ...

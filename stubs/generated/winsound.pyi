# winsound.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-4）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
from typing import Any

def Beep(frequency: Any, duration: Any) -> Any: ...
MB_ICONASTERISK: int
MB_ICONEXCLAMATION: int
MB_ICONHAND: int
MB_ICONQUESTION: int
MB_OK: int
def MessageBeep(type: Any = ...) -> Any: ...
def PlaySound(sound: Any, flags: Any) -> Any: ...
SND_ALIAS: int
SND_APPLICATION: int
SND_ASYNC: int
SND_FILENAME: int
SND_LOOP: int
SND_MEMORY: int
SND_NODEFAULT: int
SND_NOSTOP: int
SND_NOWAIT: int
SND_PURGE: int

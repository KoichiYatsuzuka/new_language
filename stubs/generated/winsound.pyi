# winsound.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/winsound.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer
from typing import Final, Literal, overload
SND_APPLICATION: int = 128
SND_FILENAME: int = 131072
SND_ALIAS: int = 65536
SND_LOOP: int = 8
SND_MEMORY: int = 4
SND_PURGE: int = 64
SND_ASYNC: int = 1
SND_NODEFAULT: int = 2
SND_NOSTOP: int = 16
SND_NOWAIT: int = 8192
MB_ICONASTERISK: int = 64
MB_ICONEXCLAMATION: int = 48
MB_ICONHAND: int = 16
MB_ICONQUESTION: int = 32
MB_OK: int = 0

def Beep(frequency: int, duration: int) -> None:
    ...

@overload
def PlaySound(sound: ReadableBuffer | None, flags: Literal[4]) -> None:
    ...

@overload
def PlaySound(sound: str | ReadableBuffer | None, flags: int) -> None:
    ...

def MessageBeep(type: int=0) -> None:
    ...

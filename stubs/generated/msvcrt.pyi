# msvcrt.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/msvcrt.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from typing import Final
CRT_ASSEMBLY_VERSION: Final[str]
LK_UNLCK: int = 0
LK_LOCK: int = 1
LK_NBLCK: int = 2
LK_RLCK: int = 3
LK_NBRLCK: int = 4
SEM_FAILCRITICALERRORS: int = 1
SEM_NOALIGNMENTFAULTEXCEPT: int = 4
SEM_NOGPFAULTERRORBOX: int = 2
SEM_NOOPENFILEERRORBOX: int = 32768

def locking(fd: int, mode: int, nbytes: int, /) -> None:
    ...

def setmode(fd: int, mode: int, /) -> int:
    ...

def open_osfhandle(handle: int, flags: int, /) -> int:
    ...

def get_osfhandle(fd: int, /) -> int:
    ...

def kbhit() -> bool:
    ...

def getch() -> bytes:
    ...

def getwch() -> str:
    ...

def getche() -> bytes:
    ...

def getwche() -> str:
    ...

def putch(char: bytes | bytearray, /) -> None:
    ...

def putwch(unicode_char: str, /) -> None:
    ...

def ungetch(char: bytes | bytearray, /) -> None:
    ...

def ungetwch(unicode_char: str, /) -> None:
    ...

def heapmin() -> None:
    ...

def SetErrorMode(mode: int, /) -> int:
    ...

def GetErrorMode() -> int:
    ...

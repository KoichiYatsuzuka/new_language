# _tkinter.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_tkinter.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import FileDescriptorLike, Incomplete
from collections.abc import Callable
from typing import Any, ClassVar, Final, Literal, TypeAlias, final, overload
from typing_extensions import deprecated

@final
class Tcl_Obj:

    @property
    def string(self) -> str:
        ...

    @property
    def typename(self) -> str:
        ...
    __hash__: ClassVar[None]

    def __eq__(self, value, /):
        ...

    def __ge__(self, value, /):
        ...

    def __gt__(self, value, /):
        ...

    def __le__(self, value, /):
        ...

    def __lt__(self, value, /):
        ...

    def __ne__(self, value, /):
        ...

class TclError(Exception):
    ...

@final
class TkappType:

    def adderrorinfo(self, msg: str, /) -> None:
        ...

    def call(self, command: Any, /, *args: Any) -> Any:
        ...

    def createcommand(self, name: str, func: Callable[..., object], /) -> None:
        ...

    def createtimerhandler(self, milliseconds: int, func: Callable[..., object], /):
        ...

    def deletecommand(self, name: str, /) -> None:
        ...

    def dooneevent(self, flags: int=0, /) -> int:
        ...

    def eval(self, script: str, /) -> str:
        ...

    def evalfile(self, fileName: str, /) -> str:
        ...

    def exprboolean(self, s: str, /) -> Literal[0, 1]:
        ...

    def exprdouble(self, s: str, /) -> float:
        ...

    def exprlong(self, s: str, /) -> int:
        ...

    def exprstring(self, s: str, /) -> str:
        ...

    def getboolean(self, arg, /) -> bool:
        ...

    def getdouble(self, arg, /) -> float:
        ...

    def getint(self, arg, /) -> int:
        ...

    def getvar(self, *args, **kwargs):
        ...

    def globalgetvar(self, *args, **kwargs):
        ...

    def globalsetvar(self, *args, **kwargs):
        ...

    def globalunsetvar(self, *args, **kwargs):
        ...

    def interpaddr(self) -> int:
        ...

    def loadtk(self) -> None:
        ...

    def mainloop(self, threshold: int=0, /) -> None:
        ...

    def quit(self) -> None:
        ...

    def record(self, script: str, /) -> str:
        ...

    def setvar(self, *ags, **kwargs):
        ...

    def splitlist(self, arg, /) -> tuple[Incomplete, ...]:
        ...

    def unsetvar(self, *args, **kwargs):
        ...

    def wantobjects(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def willdispatch(self) -> None:
        ...

    def gettrace(self, /) -> Callable[[tuple[str, ...]], object] | None:
        ...

    def settrace(self, func: Callable[[tuple[str, ...]], object] | None, /) -> None:
        ...
ALL_EVENTS: int = -3
FILE_EVENTS: int = 8
IDLE_EVENTS: int = 32
TIMER_EVENTS: int = 16
WINDOW_EVENTS: int = 4
DONT_WAIT: int = 2
EXCEPTION: int = 8
READABLE: int = 2
WRITABLE: int = 4
TCL_VERSION: Final[str]
TK_VERSION: Final[str]

@final
class TkttType:

    def deletetimerhandler(self) -> None:
        ...

def create(screenName: str | None=None, baseName: str='', className: str='Tk', interactive: bool=False, wantobjects: bool=False, wantTk: bool=True, sync: bool=False, use: str | None=None, /) -> TkappType:
    ...

def getbusywaitinterval() -> int:
    ...

def setbusywaitinterval(new_val: int, /) -> None:
    ...

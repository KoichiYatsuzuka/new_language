# _thread.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_thread.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import signal
import sys
from _typeshed import structseq
from collections.abc import Callable
from threading import Thread
from types import TracebackType
from typing import Any, Final, NoReturn, final, overload
from typing_extensions import TypeVarTuple, Unpack, deprecated, disjoint_base
error = RuntimeError

def _count() -> int:
    ...

@final
class RLock:

    def acquire(self, blocking: bool=True, timeout: float=-1) -> bool:
        ...

    def release(self) -> None:
        ...
    __enter__ = acquire

    def __exit__(self, t: type[BaseException] | None, v: BaseException | None, tb: TracebackType | None) -> None:
        ...

@final
class LockType:

    def acquire(self, blocking: bool=True, timeout: float=-1) -> bool:
        ...

    def release(self) -> None:
        ...

    def locked(self) -> bool:
        ...

    @deprecated('Obsolete synonym. Use `acquire()` instead.')
    def acquire_lock(self, blocking: bool=True, timeout: float=-1) -> bool:
        ...

    @deprecated('Obsolete synonym. Use `release()` instead.')
    def release_lock(self) -> None:
        ...

    @deprecated('Obsolete synonym. Use `locked()` instead.')
    def locked_lock(self) -> bool:
        ...

    def __enter__(self) -> bool:
        ...

    def __exit__(self, type: type[BaseException] | None, value: BaseException | None, traceback: TracebackType | None) -> None:
        ...

@overload
def start_new_thread(function: Callable[[Unpack[Any]], object], args: tuple[Unpack[Any]], /) -> int:
    ...

@overload
def start_new_thread(function: Callable[..., object], args: tuple[Any, ...], kwargs: dict[str, Any], /) -> int:
    ...

@overload
@deprecated('Obsolete synonym. Use `start_new_thread()` instead.')
def start_new(function: Callable[[Unpack[Any]], object], args: tuple[Unpack[Any]], /) -> int:
    ...

@overload
@deprecated('Obsolete synonym. Use `start_new_thread()` instead.')
def start_new(function: Callable[..., object], args: tuple[Any, ...], kwargs: dict[str, Any], /) -> int:
    ...

def interrupt_main(signum: signal.Signals=signal.SIGINT, /) -> None:
    ...

def exit() -> NoReturn:
    ...

@deprecated('Obsolete synonym. Use `exit()` instead.')
def exit_thread() -> NoReturn:
    ...

def allocate_lock() -> LockType:
    ...

@deprecated('Obsolete synonym. Use `allocate_lock()` instead.')
def allocate() -> LockType:
    ...

def get_ident() -> int:
    ...

def stack_size(size: int=0, /) -> int:
    ...
TIMEOUT_MAX: Final[float]

def get_native_id() -> int:
    ...

@final
class _ExceptHookArgs(structseq[Any], tuple[type[BaseException], BaseException | None, TracebackType | None, Thread | None]):
    __match_args__: Final = ('exc_type', 'exc_value', 'exc_traceback', 'thread')

    @property
    def exc_type(self) -> type[BaseException]:
        ...

    @property
    def exc_value(self) -> BaseException | None:
        ...

    @property
    def exc_traceback(self) -> TracebackType | None:
        ...

    @property
    def thread(self) -> Thread | None:
        ...
_excepthook: Callable[[_ExceptHookArgs], Any]

def daemon_threads_allowed() -> bool:
    ...

@disjoint_base
class _local:
    ...

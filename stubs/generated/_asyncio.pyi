# _asyncio.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_asyncio.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from asyncio.events import AbstractEventLoop
from collections.abc import Awaitable, Callable, Coroutine, Generator
from contextvars import Context
from types import FrameType, GenericAlias
from typing import Any, Literal, TextIO, TypeAlias, TypeVar
from typing_extensions import Self, disjoint_base

@disjoint_base
class Future(Awaitable[_T]):
    _state: str

    @property
    def _exception(self) -> BaseException | None:
        ...
    _blocking: bool

    @property
    def _log_traceback(self) -> bool:
        ...

    @_log_traceback.setter
    def _log_traceback(self, val: Literal[False]) -> None:
        ...
    _asyncio_future_blocking: bool

    def __init__(self, *, loop: AbstractEventLoop | None=None) -> None:
        ...

    def __del__(self) -> None:
        ...

    def get_loop(self) -> AbstractEventLoop:
        ...

    @property
    def _callbacks(self) -> list[tuple[Callable[[Self], Any], Context]]:
        ...

    def add_done_callback(self, fn: Callable[[Self], object], /, *, context: Context | None=None) -> None:
        ...

    def cancel(self, msg: Any | None=None) -> bool:
        ...

    def cancelled(self) -> bool:
        ...

    def done(self) -> bool:
        ...

    def result(self) -> Any:
        ...

    def exception(self) -> BaseException | None:
        ...

    def remove_done_callback(self, fn: Callable[[Self], object], /) -> int:
        ...

    def set_result(self, result: Any, /) -> None:
        ...

    def set_exception(self, exception: type | BaseException, /) -> None:
        ...

    def __iter__(self) -> Generator[Any, None, Any]:
        ...

    def __await__(self) -> Generator[Any, None, Any]:
        ...

    @property
    def _loop(self) -> AbstractEventLoop:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

@disjoint_base
class Task(Future[_T_co]):

    def __init__(self, coro: Coroutine[Any, Any, Any][Any], *, loop: AbstractEventLoop | None=None, name: str | None=None, context: Context | None=None, eager_start: bool=False) -> None:
        ...

    def get_coro(self) -> Coroutine[Any, Any, Any][Any] | None:
        ...

    def get_name(self) -> str:
        ...

    def set_name(self, value: object, /) -> None:
        ...

    def get_context(self) -> Context:
        ...

    def get_stack(self, *, limit: int | None=None) -> list[FrameType]:
        ...

    def print_stack(self, *, limit: int | None=None, file: TextIO | None=None) -> None:
        ...

    def cancelling(self) -> int:
        ...

    def uncancel(self) -> int:
        ...

    def __class_getitem__(cls, item: Any, /) -> GenericAlias:
        ...

def get_event_loop() -> AbstractEventLoop:
    ...

def get_running_loop() -> AbstractEventLoop:
    ...

def _set_running_loop(loop: AbstractEventLoop | None, /) -> None:
    ...

def _get_running_loop() -> AbstractEventLoop | None:
    ...

def _register_task(task: Task[Any]) -> None:
    ...

def _unregister_task(task: Task[Any]) -> None:
    ...

def _enter_task(loop: AbstractEventLoop, task: Task[Any]) -> None:
    ...

def _leave_task(loop: AbstractEventLoop, task: Task[Any]) -> None:
    ...

def current_task(loop: AbstractEventLoop | None=None) -> Task[Any] | None:
    ...

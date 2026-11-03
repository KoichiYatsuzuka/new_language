# _lsprof.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_lsprof.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from _typeshed import structseq
from collections.abc import Callable
from types import CodeType
from typing import Any, Final, final
from typing_extensions import disjoint_base

@disjoint_base
class Profiler:

    def __init__(self, timer: Callable[[], float] | None=None, timeunit: float=0.0, subcalls: bool=True, builtins: bool=True) -> None:
        ...

    def getstats(self) -> list[profiler_entry]:
        ...

    def enable(self, subcalls: bool=True, builtins: bool=True) -> None:
        ...

    def disable(self) -> None:
        ...

    def clear(self) -> None:
        ...

@final
class profiler_entry(structseq[Any], tuple[CodeType | str, int, int, float, float, list[profiler_subentry]]):
    __match_args__: Final = ('code', 'callcount', 'reccallcount', 'totaltime', 'inlinetime', 'calls')
    code: CodeType | str
    callcount: int
    reccallcount: int
    totaltime: float
    inlinetime: float
    calls: list[profiler_subentry]

@final
class profiler_subentry(structseq[Any], tuple[CodeType | str, int, int, float, float]):
    __match_args__: Final = ('code', 'callcount', 'reccallcount', 'totaltime', 'inlinetime')
    code: CodeType | str
    callcount: int
    reccallcount: int
    totaltime: float
    inlinetime: float

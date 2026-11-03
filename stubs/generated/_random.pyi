# _random.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_random.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
from typing import TypeAlias
from typing_extensions import disjoint_base

@disjoint_base
class Random:

    def __init__(self, seed: object=..., /) -> None:
        ...

    def seed(self, n: object=None, /) -> None:
        ...

    def getstate(self) -> tuple[int, ...]:
        ...

    def setstate(self, state: tuple[int, ...], /) -> None:
        ...

    def random(self) -> float:
        ...

    def getrandbits(self, k: int, /) -> int:
        ...

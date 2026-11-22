# py_typing_protocols.py — `typing` / `collections.abc` の型を書いた関数とクラス（examples/interop/py_typing_protocols.ar）

import collections.abc as cabc
from collections.abc import Iterator, Sequence
from typing import Iterable, Mapping, SupportsIndex, overload


class Bag(Sequence):
    """`Sequence` の抽象クラスを継承する（`__contains__` / `__iter__` / `count` などは抽象クラスが足す）。"""

    def __init__(self, items):
        self.items = items

    def __getitem__(self, i):
        return self.items[i]

    def __len__(self):
        return len(self.items)


class Table(cabc.Mapping):
    """`collections.abc.Mapping` を継承する。"""

    def __getitem__(self, key):
        return len(key)

    def __len__(self):
        return 2

    def __iter__(self):
        return iter(["a", "bb"])


def bag() -> Bag:
    return Bag([1, 2, 3])


def table() -> Table:
    return Table()


def at(i: SupportsIndex) -> int:
    return [10, 20, 30][i]


def total(xs: Sequence[int]) -> int:
    return sum(xs)


def joined(words: Iterable[str]) -> str:
    return "-".join(words)


def lookup(m: Mapping[str, int], key: str) -> int:
    return m[key]


def countdown(n: int) -> Iterator[int]:
    return iter(range(n, 0, -1))


@overload
def pick(x: int) -> int: ...
@overload
def pick(x: str) -> str: ...
def pick(x):
    """`@overload` の形は引数の型で選ぶ（int なら int・str なら str）。"""
    return x

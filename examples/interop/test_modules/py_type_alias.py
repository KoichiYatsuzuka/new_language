"""py_type_alias.py — 型の別名（`type X = T`・`X = 型の式`・`X: TypeAlias`）を使う関数（py_type_alias.ar が CPython で動かす）。"""

from typing import TypeAlias, TypeVar

from test_modules.py_type_alias_defs import Grid, Scalar

type Vec = list[int]
type Name = str
type Pair = Vec  # エイリアスの連鎖（登録時に右辺を解決するので list[int] になる）


def total(xs: Vec) -> int:
    s = 0
    for v in xs:
        s = s + v
    return s


def greet(n: Name) -> Name:
    return "hi " + n


def chained(p: Pair) -> int:
    return len(p)


_T = TypeVar("_T")
Couple = tuple[_T, _T]  # 型変数を持つ別名（`Couple[str]`）
Count: TypeAlias = int


def swap(p: Couple[str]) -> Couple[str]:
    return (p[1], p[0])


def tally(n: Count) -> Count:
    return n + 1


def cell(g: Grid, i: int) -> Scalar:
    return g[i][i]

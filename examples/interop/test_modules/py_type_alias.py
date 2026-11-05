"""py_type_alias.py — 型エイリアス（`type X = T`・Python 3.12+）を使う関数（py_type_alias.ar が CPython で動かす）。"""

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

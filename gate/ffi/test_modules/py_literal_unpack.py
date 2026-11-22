"""py_literal_unpack.py — リテラルの中の `*` 展開を使う関数（py_literal_unpack.ar が CPython で動かす）。"""


def mid(a):
    return [0, *a, 9]


def head(a):
    return [*a, 9]


def only(a):
    return [*a]


def two(a, b):
    return [*a, *b]


def from_tuple(t):
    # ★ 以前はここが `TypeError: unsupported operand types for Add: tuple and list` だった。
    return [*t, 9]


def copy_check(a):
    b = [*a]
    b.append(99)
    return (len(a), len(b))


def tup(a):
    # ★ タプル表示の展開（以前は明示エラー）。
    return (*a, 9)


def st(a):
    # ⚠ セットの並びは当てにしない。要素数と所属で確認する。
    u = {*a, 9}
    return (len(u), 9 in u)

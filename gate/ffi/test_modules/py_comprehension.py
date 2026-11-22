"""py_comprehension.py — 内包表記（リスト・集合・辞書）を使う関数（py_comprehension.ar が CPython で動かす）。"""


def simple(xs):
    return [v * 2 for v in xs]


def filtered(xs):
    return [v for v in xs if v > 2]


def two_ifs(xs):
    return [v for v in xs if v > 1 if v < 4]


def nested(xs, ys):
    """多重 for。結果は入れ子ではなく平坦な 1 本のリスト。"""
    return [a * b for a in xs for b in ys]


def nested_filtered(xs, ys):
    return [a * b for a in xs if a > 2 for b in ys if b > 10]


def over_range(n):
    return [v for v in range(n)]


def expr_elt(xs):
    return [str(v) + "!" for v in xs]


def comp_of_comp(xs, ys):
    """要素側がリスト内包（入れ子のリストになる）。"""
    return [[a for a in xs] for b in ys]


def in_call(xs):
    return len([v for v in xs if v > 1])


def set_comp_size(xs):
    """セット内包。⚠ 表示順は当てにできないので大きさで確認する。"""
    return len({v % 2 for v in xs})


def set_comp_has(xs, k):
    return k in {v % 2 for v in xs}


def dict_comp(ks):
    """辞書内包。"""
    return {k: k * 2 for k in ks if k != 3}


def dict_comp_items(d):
    """値を入れ替えた辞書。キーの順は元の順（CPython の挿入順と同じ）。"""
    return {v: k for k, v in d.items()}

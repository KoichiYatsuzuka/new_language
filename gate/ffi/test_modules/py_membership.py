"""py_membership.py — `in` / `not in` を使う関数（py_membership.ar が CPython で動かす）。"""


def in_list(t, xs):
    return t in xs


def not_in_list(t, xs):
    return t not in xs


def in_dict(k, d):
    """dict は**キー**に対するメンバシップ（Python と同じ）。"""
    return k in d


def in_str(sub, s):
    """str は**部分文字列**（要素ではない）。"""
    return sub in s


def in_tuple(t, tup):
    return t in tup


def in_set(t, st):
    return t in st


def guard(xs):
    """条件式としての利用。"""
    if 2 in xs:
        return "yes"
    return "no"


def combined(t, xs, ys):
    """`and` と組み合わせる。"""
    return t in xs and t not in ys


def filter_loop(xs, banned):
    """典型的な用途: ループ内フィルタ。"""
    out = []
    for v in xs:
        if v not in banned:
            out.append(v)
    return out

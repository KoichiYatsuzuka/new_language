"""py_multi_assign.py — 複数代入とタプルのアンパック代入を使う関数（py_multi_assign.ar が CPython で動かす）。"""

CALLS = []


def bump():
    CALLS.append(1)
    return 1


def chain():
    a = b = 5
    return (a, b)


def chain_once():
    # 右辺が **1 回だけ** 評価されること（一時変数の効果）。
    a = b = bump()
    return (a, b, len(CALLS))


def unpack2(t):
    a, b = t
    return (a, b)


def unpack3(t):
    a, b, c = t
    return a + b + c


def unpack_rest(t):
    a, *rest = t
    return (a, len(rest))


def unpack_list(t):
    [a, b] = t
    return (a, b)


def swap(x, y):
    x, y = y, x
    return (x, y)


def in_loop(pairs):
    out = []
    for p in pairs:
        a, b = p
        out.append(a + b)
    return out


def in_while(t, n):
    total = 0
    i = 0
    while i < n:
        a, b = t
        total = total + a + b
        i = i + 1
    return total


def attr_targets(t):
    d = {}
    d["a"], d["b"] = t
    return d


def nested(t):
    # ★ 入れ子のアンパック（再帰で段ごとに一時変数を取る）
    a, (b, c) = t
    return (a, b, c)


def nested_deep(t):
    a, (b, (c, d)) = t
    return (a, b, c, d)


def nested_head(t):
    (a, b), c = t
    return (a, b, c)


def nested_rest(t):
    a, (b, *r) = t
    return (a, b, len(r))

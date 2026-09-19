"""py_multi_assign.py — 複数代入（項目 15）とタプルアンパック代入（U2）。

どちらも「**この文の直前に補助文を置く**」機構（INF-B / INF-C）で脱糖する:

    a = b = c     ->  __py_tmp_N = c ; a = __py_tmp_N ; b = __py_tmp_N
    a, b = t      ->  __py_tmp_N = t ; a = __py_tmp_N[0] ; b = __py_tmp_N[1]
    a, *rest = t  ->  __py_tmp_N = t ; a = __py_tmp_N[0] ; rest = __py_tmp_N[1:]

⚠ 一時変数は**必須**。直接 `a = c; b = c` と書くと右辺が 2 回評価される。
"""

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

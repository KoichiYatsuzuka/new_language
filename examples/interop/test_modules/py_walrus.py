"""py_walrus.py — walrus 演算子 `:=` を使う関数（py_walrus.ar が CPython で動かす）。"""

CALLS = []


def f(v):
    CALLS.append(v)
    return v


def in_if(xs):
    if (n := len(xs)) > 2:
        return n * 10
    return n


def in_return(x):
    return (y := x + 1) + y


def in_call(x):
    return len([(y := x - 10)]) + y


def in_assign(x):
    z = (y := x + 1) * 2
    return (y, z)


def once():
    # `f` が **1 回だけ** 呼ばれること（持ち上げの効果）。
    r = (a := f(7)) + a
    return (r, len(CALLS))

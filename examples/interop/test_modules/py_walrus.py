"""py_walrus.py — walrus 演算子 `:=`（項目 23）。

`(x := expr)` を「**この文の直前に** `x = expr` を置き、式は `x` を返す」形へ脱糖する
（INF-B）。⇒ 右辺は **1 回だけ**評価される（`once()` が固定）。

⚠ 持ち上げると評価回数が変わる位置（`while` の条件・`and`/`or` の右辺・三項の腕・
  内包表記の中）は**明示エラー**。→ `py_walrus_error.ar`。
"""

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

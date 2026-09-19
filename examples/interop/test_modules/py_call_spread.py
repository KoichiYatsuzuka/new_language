"""py_call_spread.py — 呼び出し引数の展開 `f(*xs)` / `f(**d)`（群7 U4 / U5）。

Arrow 側にも同じ構文（`CallArg::Spread` / `CallArg::KwSpread`）を入れたので
1 対 1 で写せる。

⚠ **転送パターン**（`def forward(*args, **kwargs): return g(*args, **kwargs)`）が
  これで成立する —— Python で最頻出の形のひとつ。
"""


def add3(a, b, c):
    return a + b + c


def star(xs):
    return add3(*xs)


def mixed(x, pair):
    return add3(x, *pair)


def kwstar(d):
    return add3(1, **d)


def forward(*args, **kwargs):
    # ★ 受けた引数をそのまま別の関数へ渡す（転送パターン）
    return add3(*args, **kwargs)


def use_forward():
    return forward(1, 2, c=3)

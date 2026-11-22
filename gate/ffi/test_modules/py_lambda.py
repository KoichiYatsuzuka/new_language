"""py_lambda.py — lambda を使う関数（py_lambda.ar が CPython で動かす）。"""


def apply(f, x):
    return f(x)


def simple(x):
    g = lambda v: v + 1
    return g(x)


def capture(n):
    # クロージャ捕捉（持ち上げ先が同じスコープなので効く）。
    g = lambda v: v + n
    return g(10)


def as_arg(x):
    # 呼び出し引数の中（`sorted(xs, key=lambda ...)` と同じ位置）。
    return apply(lambda v: v * 3, x)


def with_default(x):
    g = lambda v, k=10: v + k
    return (g(x), g(x, 1))


def nested(x):
    # 内側の lambda は**外側の本体の中**へ持ち上がる。
    outer = lambda a: (lambda b: a + b)
    inner = outer(x)
    return inner(100)


def two_in_one(x):
    a = lambda v: v + 1
    b = lambda v: v * 2
    return (a(x), b(x))


def as_default(k=lambda v: v + 1):
    # デフォルト値の中の lambda は `def` の前へ持ち上がる。
    return k(5)

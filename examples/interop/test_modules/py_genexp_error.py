"""py_genexp_error.py - ジェネレータ式は遅延評価なので未対応（項目 17）。"""


def f(xs):
    return sum(v for v in xs)

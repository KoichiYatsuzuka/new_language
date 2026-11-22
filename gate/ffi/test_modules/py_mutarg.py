"""py_mutarg.py — 引数を受ける関数（変数・リテラル・要素・戻り値を渡す）（py_mutarg.ar が CPython で動かす）。"""


def take(v):
    return v


def scale(factor, xs):
    out = []
    for x in xs:
        out.append(x * factor)
    return out

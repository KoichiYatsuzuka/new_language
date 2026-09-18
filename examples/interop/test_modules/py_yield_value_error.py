"""py_yield_value_error.py — 値位置の `yield`（変換器が明示エラーにする形）。

`x = yield 1` は双方向通信（`.send()`）の構文。Arrow の `yield` は文で、
値を受け取る形が無い。
"""


def g():
    x = yield 1
    return x

"""py_lambda_and_error.py — `and` の右辺の lambda（明示エラー）。

条件付き評価の位置。walrus と同じガード（`UnsafeHoistGuard`）で止めている。
"""


def g(a):
    return a and (lambda: 1)

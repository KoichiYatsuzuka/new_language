"""py_unpack_midstar_error.py — 途中の `*`（変換器が明示エラーにする形）。

`a, *m, b = t` は `b` を**末尾から**数える必要がある（`m` の長さが実行時まで
決まらない）。末尾の `*rest` だけスライスで受ける。
"""


def g(t):
    a, *m, b = t
    return (a, b)

"""py_spread_notiter_error.py — 反復できない値の `*` 展開（静的エラー）。

⚠ 展開元の型が判れば**静的に**捕まる（`SpreadArgNotIterable`）。
判らないとき（`Any` / `Unresolved`）は通し、実行時に落ちる。
"""


def g():
    n = 5
    return [*n]

"""py_walrus_and_error.py — `and` の右辺での walrus（明示エラー）。

`a` が偽なら右辺は**評価されない**。持ち上げると必ず評価されてしまう。
`or` の右辺・三項の腕も同じ理由で拒否する。
"""


def g(a, xs):
    if a and (n := len(xs)) > 0:
        return n
    return 0

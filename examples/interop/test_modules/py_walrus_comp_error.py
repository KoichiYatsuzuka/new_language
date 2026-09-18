"""py_walrus_comp_error.py — 内包表記の中での walrus（明示エラー）。

フィルタは**要素ごと**に評価される。持ち上げると 1 回だけになる。
"""


def g(xs):
    return [v for v in xs if (n := v) > 1]

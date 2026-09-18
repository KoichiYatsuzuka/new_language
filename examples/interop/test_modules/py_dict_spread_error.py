"""py_dict_spread_error.py — 辞書リテラルの `**` 展開（変換時エラー）。

Arrow に dict のマージ手段が無い（`|` も `update()` も無い）。
`dict()` コンストラクタが入れば `dict(d1.items() + d2.items())` で書ける。
"""


def g(d):
    return {**d, "z": 1}

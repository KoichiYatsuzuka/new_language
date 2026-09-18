"""py_lambda_comp_error.py — 内包表記の中の lambda（明示エラー）。

持ち上げ先は**文の直前**なので、内包表記のループ変数はそこでは見えない。
`[(lambda: x) for x in xs]` の `x` がスコープ外になる。
"""


def g(xs):
    return [(lambda: x) for x in xs]

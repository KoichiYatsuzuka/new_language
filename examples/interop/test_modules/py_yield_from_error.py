"""py_yield_from_error.py — `yield from`（変換器が明示エラーにする形）。

「内側のジェネレータへ委譲する」構文で、Arrow に相当するものが無い。
`for v in xs: yield v` へ機械的に書き換えると `.send()` と戻り値の意味が
変わるので、黙って変えずに止める。
"""


def g(xs):
    yield from xs

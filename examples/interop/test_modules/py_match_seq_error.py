"""py_match_seq_error.py — シーケンスパターン（変換器が明示エラーにする形）。

Arrow の `match` は値の等価比較だけで、構造を分解できない。
"""


def g(v):
    match v:
        case [a, b]:
            return a
        case _:
            return None

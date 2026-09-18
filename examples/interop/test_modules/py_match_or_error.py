"""py_match_or_error.py — ORパターン（変換器が明示エラーにする形）。

Arrow の `match` は値の等価比較だけで、構造を分解できない。
"""


def g(n):
    match n:
        case 1 | 2:
            return 'low'
        case _:
            return 'hi'

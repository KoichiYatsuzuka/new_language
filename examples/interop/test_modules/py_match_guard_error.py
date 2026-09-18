"""py_match_guard_error.py — ガードパターン（変換器が明示エラーにする形）。

Arrow の `match` は値の等価比較だけで、構造を分解できない。
"""


def g(n):
    match n:
        case 1 if n > 0:
            return 'pos'
        case _:
            return 'no'

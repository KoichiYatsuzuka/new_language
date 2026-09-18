"""py_match_class_error.py — クラスパターン（変換器が明示エラーにする形）。

Arrow の `match` は値の等価比較だけで、構造を分解できない。
"""


class C:
    pass

def g(v):
    match v:
        case C():
            return 1
        case _:
            return 0

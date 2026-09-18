"""py_match_capture_error.py — キャプチャパターン（変換器が明示エラーにする形）。

Arrow の `match` は値の等価比較だけで、構造を分解できない。
"""


def g(n):
    match n:
        case x:
            return x

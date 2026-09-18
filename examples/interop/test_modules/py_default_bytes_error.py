"""py_default_bytes_error.py — bytes リテラルのデフォルト式（明示エラー）。

Arrow に bytes 型が無い。項目 1 でデフォルト式も変換するようになったので、
中身の未対応がここで表に出る。
"""


def g(b=b"xy"):
    return b

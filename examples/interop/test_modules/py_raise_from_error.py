"""py_raise_from_error.py — `raise X from Y`（変換器が明示エラーにする形）。

例外連鎖（`__cause__`）は Arrow に無い。黙って捨てると原因情報が消える。
"""


def wrap(e):
    raise ValueError("wrapped") from e

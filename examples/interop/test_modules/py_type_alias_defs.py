"""py_type_alias_defs.py — ほかのモジュールが import する型の別名（`py_type_alias.py` が使う）。"""

from typing import Union

Scalar = Union[int, float]
Grid = list[list[Scalar]]  # 別名を使う別名

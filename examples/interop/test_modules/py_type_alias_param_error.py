"""py_type_alias_param_error.py — 型パラメータ付きエイリアス（変換器が明示エラーにする形）。

展開先に型変数が残るので、変換器の透過展開では扱えない。
"""

type L[T] = list[T]


def g(x: L) -> int:
    return 1

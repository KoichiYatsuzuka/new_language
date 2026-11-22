"""py_type_alias_other.py — `py_type_alias.py` と**同名**のエイリアスを別の型で定義する。

エイリアス表がモジュールをまたいで漏れていないことの確認用。
"""

type Vec = str


def size(s: Vec) -> int:
    return len(s)

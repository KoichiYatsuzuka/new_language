"""py_dict_spread.py — 辞書の合成 `{**a, **b}`（群7 U3 の dict 部分）。

Arrow 側にも同じ構文（`DictEntry::Spread`）を入れたので、1 対 1 で写せる。
⚠ **後から来たキーが勝つ**のも Python と同じ。
"""


def merge(a, b):
    return {**a, **b}


def with_extra(d):
    return {**d, "z": 9}


def before(d):
    return {"z": 0, **d}

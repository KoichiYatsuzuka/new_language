"""py_literal_unpack.py — リテラル内の `*` 展開（U3）。

Arrow に splat が無いので**連結**へ脱糖する:
    [0, *a, 9]  ->  [0] + a + [9]
    {1, *a, 2}  ->  set([1, 2]).union(a)

⚠ リスト版は**展開元がリストでないと落ちる**（Arrow に `list()` 組込みが無く
  `list(a) + [...]` と書けないため）。タプル等を渡すと `TypeError` で**大きな音で**
  止まるので、黙って違う結果にはならない。
⚠ セット版は展開元がリストでもセットでも通る（`set()` は任意のイテラブルを受け、
  `union` もリストを受ける）。
"""


def mid(a):
    return [0, *a, 9]


def head(a):
    return [*a, 9]


def tail(a):
    return [0, *a]


def only(a):
    return [*a]


def two(a, b):
    return [*a, *b]


def copy_check(a):
    b = [*a]
    b.append(99)
    return (len(a), len(b))


def s_mid(a):
    return {1, *a, 2}


def s_two(a, b):
    return {*a, *b}

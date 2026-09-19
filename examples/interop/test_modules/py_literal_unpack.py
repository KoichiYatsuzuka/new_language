"""py_literal_unpack.py — リテラル内の `*` 展開（U3）。

⚠⚠ **以前は連結へ脱糖していた**（`[0, *a, 9]` → `[0] + a + [9]`）。Arrow に splat が
無かったため。その形は**展開元がリストに限られる**という穴があり、タプルやセットを
展開すると `TypeError` になっていた。
⇒ **Arrow 側に `*other` を入れた**ので 1 対 1 で写せるようになり、
  展開元の種類を選ばなくなった（③ がその証拠）。
"""


def mid(a):
    return [0, *a, 9]


def head(a):
    return [*a, 9]


def only(a):
    return [*a]


def two(a, b):
    return [*a, *b]


def from_tuple(t):
    # ★ 以前はここが `TypeError: unsupported operand types for Add: tuple and list` だった。
    return [*t, 9]


def copy_check(a):
    b = [*a]
    b.append(99)
    return (len(a), len(b))


def tup(a):
    # ★ タプル表示の展開（以前は明示エラー）。
    return (*a, 9)


def st(a):
    # ⚠ セットの並びは当てにしない。要素数と所属で確認する。
    u = {*a, 9}
    return (len(u), 9 in u)

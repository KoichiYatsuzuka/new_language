"""py_setcomp_error.py - 集合内包はセットリテラルとは別ノードで、まだ変換できない（項目 22 / 17）。"""


def comp(xs):
    return {x * 2 for x in xs}

# chain_util.py — `import[py]` の**再帰ロード**（項目 27）の中段。
# `import` と `from ... import` の**両方**を使い、どちらの経路でも
# 依存先が読まれることを見る。

import chain_leaf
from chain_leaf import double as dbl

UTIL_MARKER = "util:" + chain_leaf.LEAF_MARKER


def quad(x):
    # `from ... import` で持ち込んだ別名
    return dbl(dbl(x))


def leaf_of(x):
    # `import` で持ち込んだ名前空間経由
    return chain_leaf.double(x)

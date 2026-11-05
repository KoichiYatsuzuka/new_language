# chain_util.py — Python のモジュールの中の import（3 段）の中段（py_import_chain.ar が CPython で動かす）。

import chain_leaf
from chain_leaf import double as dbl

UTIL_MARKER = "util:" + chain_leaf.LEAF_MARKER


def quad(x):
    # `from ... import` で持ち込んだ別名
    return dbl(dbl(x))


def leaf_of(x):
    # `import` で持ち込んだ名前空間経由
    return chain_leaf.double(x)

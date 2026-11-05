# chain_top.py — Python のモジュールの中の import（3 段）の入口（py_import_chain.ar が CPython で動かす）。

import chain_util as cu

TOP_MARKER = "top:" + cu.UTIL_MARKER


def run(x):
    return cu.quad(x) + cu.leaf_of(x)

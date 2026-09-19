# chain_top.py — `import[py]` の**再帰ロード**（項目 27）の入口。
# `chain_util` → `chain_leaf` と 2 段たどれることを見る。

import chain_util as cu

TOP_MARKER = "top:" + cu.UTIL_MARKER


def run(x):
    return cu.quad(x) + cu.leaf_of(x)

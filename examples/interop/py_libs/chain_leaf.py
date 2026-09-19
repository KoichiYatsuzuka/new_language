# chain_leaf.py — `import[py]` の**再帰ロード**（項目 27）の葉。
# 誰も import しない。ここで定義した名前が 2 段上まで届くかを見る。

LEAF_MARKER = "leaf"


def double(x):
    return x * 2

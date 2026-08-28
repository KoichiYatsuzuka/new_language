"""py_set.py - `import[py]` 変換器のセットリテラル対応（項目 22）の検査用モジュール。

Arrow にも `Expr::Set` が実在するので、`{1, 2, 3}` はそのまま写せる。
⚠ 集合内包 `{x for x in xs}` は `SetComp` という**別ノード**で、内包表記（項目 17）の担当。
"""


# --- 1. リテラル / 重複除去 / 単要素 ---
def literal():
    return {1, 2, 3}


def dedup():
    return {1, 2, 3, 2, 1}


def single():
    return {7}


# --- 2. 空セットは `set()`（`{}` は空辞書。Python と同じ） ---
def empty():
    return set()


# --- 3. メンバシップ / 長さ / 追加 ---
def membership(t):
    s = {1, 2, 3}
    return t in s


def size():
    return len({1, 2, 3, 3})


def add():
    s = {1, 2}
    s.add(3)
    return s


# --- 4. 要素が式でもよい ---
def from_exprs(a, b):
    return {a + 1, b * 2, a + 1}


# --- 5. 入れ子・tuple 要素 ---
def nested_in_list():
    return [{1, 2}, {3}]


def set_of_tuples():
    return {(1, 2), (3, 4)}


# --- 6. str のセット（repr 順は不定なのでメンバシップで確認する） ---
def str_set_membership(k):
    s = {"b", "a", "c"}
    return k in s

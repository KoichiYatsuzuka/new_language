"""py_tuple.py — タプルを使う関数（py_tuple.ar が CPython で動かす）。"""


# --- 1. リテラル / 空 / 単要素 ---
def literal():
    return (1, 2, 3)


def empty():
    return ()


def single():
    return (1,)


# --- 2. 入れ子 / 型混在 ---
def nested():
    return ((1, 2), (3, 4))


def mixed():
    return (1, "a", True, None)


# --- 3. 要素が式 ---
def from_exprs(a, b):
    return (a + 1, b * 2)


# --- 4. 添字 / スライス / len / in / 反復 ---
def index(t):
    return t[1]


def slice_it(t):
    return t[1:3]


def length(t):
    return len(t)


def contains(t, v):
    return v in t


def iterate(t):
    total = 0
    for v in t:
        total = total + v
    return total


# --- 5. デフォルト引数 / リスト要素 / 戻り値のペア ---
def as_default(t=(7, 8)):
    return t


def in_list():
    return [(1, 2), (3, 4)]


def as_return_pair(a, b):
    return (a, b)


# --- 6. 等価比較は「値」で行われる（list とは違う。下の注記参照） ---
def compare():
    return (1, 2) == (1, 2)

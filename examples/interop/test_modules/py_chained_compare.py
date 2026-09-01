"""py_chained_compare.py - `import[py]` 変換器の連鎖比較対応（項目 16）の検査用モジュール。

Python の `a < b < c` は Arrow に相当構文が無いので、隣接ペアを `and` で連結する:
  `a < b < c` → `(a < b) and (b < c)`

⚠ Arrow の `and` も短絡するので**短絡の挙動は一致**する。違うのは
  **中間オペランドを 2 回評価する**ことだけ（⑦ で固定）。
"""

CALLS = []


def reset():
    while len(CALLS) > 0:
        CALLS.pop()


# --- 1. 基本形 ---
def simple(x):
    return 1 < x < 10


# --- 2. 3 段の連鎖 ---
def triple(x, y):
    return 0 < x < y < 100


# --- 3. 演算子が混ざってもよい ---
def mixed_ops(x):
    return 0 <= x < 10


def eq_chain(a, b, c):
    return a == b == c


# --- 4. 他の演算子と組み合わせる ---
def with_in(x, xs):
    return 0 < x < 10 and x in xs


# --- 5. 単項の比較（連鎖でない）も同じ経路を通る ---
def is_chain(x):
    return x is not None


# --- 6. 条件式としての利用 ---
def guard(x):
    if 1 < x < 5:
        return "mid"
    return "out"


# --- 7. ★短絡は一致する / 中間オペランドは 2 回評価される ---
def side_len():
    CALLS.append("h")
    return 1000


def short_circuit(x):
    """`100 < x` が偽なので右側（`side_len()`）は評価されない ⇒ CALLS は 0 件。"""
    reset()
    r = 100 < x < side_len()
    return (r, len(CALLS))


def mid():
    CALLS.append("m")
    return 5


def middle_twice():
    """★中間オペランド `mid()` が Arrow では 2 回、CPython では 1 回呼ばれる。"""
    reset()
    r = 0 < mid() < 100
    return (r, len(CALLS))

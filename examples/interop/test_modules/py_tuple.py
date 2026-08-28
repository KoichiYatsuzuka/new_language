"""py_tuple.py - `import[py]` 変換器のタプル対応（項目 18）の検査用モジュール。

⚠ 項目 18 の対象は `convert_constant` の `Constant::Tuple` アームだが、そこは
**現在の構成では到達しない**（`Constant::Tuple` を作るのは rustpython の
`ConstantOptimizer` だけで、`Suite::parse` は畳み込みをしない）。
通常のタプルは常に `py::Expr::Tuple` として来る。このモジュールはその**実際に通る経路**を固定する。
"""


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

"""py_slice.py - `import[py]` 変換器のスライス対応（項目 4）の検査用モジュール。

rustpython の `ExprSlice` は `lower` / `upper` / `step` を 3 つとも `Option` で持ち、
省略された部分は `None` になる。Arrow の `Expr::Slice { begin, end, step }` も同じ形なので
そのまま写せる。
"""


# --- 1. 基本形 ---
def basic(xs):
    return xs[1:3]


def open_begin(xs):
    return xs[:2]


def open_end(xs):
    return xs[2:]


def full(xs):
    return xs[:]


# --- 2. ステップ ---
def step(xs):
    return xs[::2]


def begin_end_step(xs):
    return xs[1:4:2]


# --- 3. 負のインデックス・負のステップ ---
def negative(xs):
    return xs[:-1]


def neg_begin(xs):
    return xs[-2:]


def reverse(xs):
    return xs[::-1]


def neg_step_partial(xs):
    return xs[4:1:-1]


# --- 4. 範囲外・逆転した境界（Python は空リスト／切り詰め） ---
def out_of_range(xs):
    return xs[10:20]


def reversed_bounds(xs):
    return xs[3:1]


def big_end(xs):
    return xs[1:100]


# --- 5. str / tuple にも効く ---
def on_str(s):
    return s[1:3] + "|" + s[::-1]


def on_tuple(t):
    return t[1:3]


# --- 6. 境界が式（定数でなくてよい） ---
def computed(xs, a, b):
    return xs[a:b]


# --- 7. スライスの結果にさらにスライス ---
def nested_expr(xs):
    return xs[1:][:2]


# --- 8. ★スライス代入（項目 3 の添字代入と組み合わさって成立する） ---
def slice_assign(xs):
    xs[1:3] = [99, 98]
    return xs

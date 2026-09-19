"""py_chained_compare.py - `import[py]` 変換器の連鎖比較対応（項目 16 / D6）の検査用モジュール。

Python の `a < b < c` は Arrow に相当構文が無いので脱糖する。脱糖は 2 通りある:

  ① 中間オペランドに副作用が無い（素の名前・定数）
     → 隣接ペアを `and` で連結: `a < b < c` → `(a < b) and (b < c)`
     ⚠ 中間を 2 回読むが、読んでも観測できないので命令を増やさない。

  ② 中間オペランドに副作用がある（呼び出し・添字・属性 …）
     → **早期脱出のブロック式**（D6）:
        block->bool:
            mut t = f()
            if not (a < t):
                block_return False      # ← ここで抜けるので c は評価されない
            block_return t < c
     ⚠ 中間は 1 回だけ評価され、短絡も評価順序も CPython と一致する。

⚠ ブロック式は**その場に埋め込む**ので、`while` の条件・`and` の右辺・三項の腕・
  内包表記の中でも評価位置が動かない（⑧〜⑫ で固定）。
"""

CALLS = []


def reset():
    while len(CALLS) > 0:
        CALLS.pop()


def v(tag, value):
    """副作用のある中間オペランド。呼ばれた順に tag を記録する。"""
    CALLS.append(tag)
    return value


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


# --- 7. ★短絡は一致する ---
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


# --- 8. ★中間オペランドは 1 回だけ評価される（D6） ---
def middle_once():
    """`0 < mid() < 100` の `mid()` は 1 回だけ呼ばれる（CPython と同じ）。"""
    reset()
    r = 0 < mid() < 100
    return (r, len(CALLS))


def four_operands():
    """中間が 2 つ（`b` と `c`）。どちらも 1 回だけ。"""
    reset()
    r = 0 < v("b", 5) < v("c", 50) < 100
    return (r, len(CALLS))


def early_false():
    """先頭の比較が偽 ⇒ `c` は評価されない（`b` の 1 件だけ）。"""
    reset()
    r = 100 < v("b", 5) < v("c", 50) < 1000
    return (r, len(CALLS))


def eval_order():
    """評価順序は左→右。**先頭にも副作用がある**形で固定する。"""
    reset()
    r = v("a", 0) < v("b", 5) < v("c", 100)
    return (r, CALLS[0] + CALLS[1] + CALLS[2])


# --- 9. ★埋め込み位置が動かないこと ---
def in_while():
    """`while` の条件 ⇒ **毎周回**評価される（持ち上げていたら 1 回で終わる）。"""
    reset()
    n = 0
    while 0 < v("w", 3) < 100 and n < 3:
        n = n + 1
    return (n, len(CALLS))


def in_and_rhs():
    """`and` の右辺 ⇒ 左が偽なら評価されない。"""
    reset()
    r = False and (0 < v("x", 5) < 10)
    return (r, len(CALLS))


def in_or_rhs():
    """`or` の右辺 ⇒ 左が真なら評価されない。"""
    reset()
    r = True or (0 < v("y", 5) < 10)
    return (r, len(CALLS))


def in_ternary():
    """三項の**選ばれない腕** ⇒ 評価されない。"""
    reset()
    r = "hit" if True else ("no" if 0 < v("z", 5) < 10 else "no2")
    return (r, len(CALLS))


def in_comprehension():
    """内包表記の条件 ⇒ **要素ごとに 1 回**（3 要素で 3 件）。"""
    reset()
    r = [x for x in [1, 2, 3] if 0 < v("c", x) < 3]
    return (r, len(CALLS))


# --- 10. ★同一性が動かないこと（一時変数を作っても `is` は壊れない） ---
CACHE = [1]


def get():
    return CACHE


def is_identity():
    return get() is CACHE is get()

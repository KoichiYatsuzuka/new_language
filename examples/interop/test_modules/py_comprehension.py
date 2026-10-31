"""py_comprehension.py - `import[py]` 変換器の内包表記対応（項目 17）の検査用モジュール。

★ 脱糖は Arrow のネイティブ内包表記と**同じ関数**（`ast::build_list_comprehension`）を通る。
⇒ 「Arrow で書いた内包表記」と「Python 由来の内包表記」で AST が食い違うことはない。
"""


def simple(xs):
    return [v * 2 for v in xs]


def filtered(xs):
    return [v for v in xs if v > 2]


def two_ifs(xs):
    return [v for v in xs if v > 1 if v < 4]


def nested(xs, ys):
    """多重 for。結果は入れ子ではなく平坦な 1 本のリスト。"""
    return [a * b for a in xs for b in ys]


def nested_filtered(xs, ys):
    return [a * b for a in xs if a > 2 for b in ys if b > 10]


def over_range(n):
    return [v for v in range(n)]


def expr_elt(xs):
    return [str(v) + "!" for v in xs]


def comp_of_comp(xs, ys):
    """要素側がリスト内包（入れ子のリストになる）。"""
    return [[a for a in xs] for b in ys]


def in_call(xs):
    return len([v for v in xs if v > 1])


def set_comp_size(xs):
    """セット内包。⚠ 表示順は当てにできないので大きさで確認する。"""
    return len({v % 2 for v in xs})


def set_comp_has(xs, k):
    return k in {v % 2 for v in xs}


def dict_comp(ks):
    """辞書内包（`dict([(k, v) for ...])` に脱糖される・python_builtins_plan.md のタスク 3-1）。"""
    return {k: k * 2 for k in ks if k != 3}


def dict_comp_items(d):
    """値を入れ替えた辞書。キーの順は元の順（CPython の挿入順と同じ）。"""
    return {v: k for k, v in d.items()}

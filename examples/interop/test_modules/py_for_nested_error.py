"""py_for_nested_error.py — 入れ子タプルの for ターゲット（明示エラー）。

Arrow の `Stmt::For.targets` は `Vec<String>`（単純名の並び）なので、
入れ子の分解を表す受け皿が無い。
"""


def g(rows):
    out = []
    for a, (b, c) in rows:
        out.append(a + b + c)
    return out

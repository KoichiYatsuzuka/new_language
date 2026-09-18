"""py_mutarg.py — `import[py]` 由来の関数へ**一時値**を渡す形の検査（INF-D）。

変換器は Python に不変引数の概念が無いため全パラメータを `mutable: true` にする
（coverage の 🔵「変換パラメータは常に mutable: true」）。そのため呼び出し側の
可変性検査が、ネイティブ `fn` と**同じ規則**である必要がある。
"""


def take(v):
    return v


def scale(factor, xs):
    out = []
    for x in xs:
        out.append(x * factor)
    return out

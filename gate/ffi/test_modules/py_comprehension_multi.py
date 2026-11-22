"""py_comprehension_multi.py — 内包表記の多ターゲット（`[k for k, v in d.items()]`）。

⚠ 以前は `tuple unpacking in a list comprehension target is not supported` だった。
Arrow 側の `ComprehensionClause.targets` / `ForExpr.targets` を `Vec<String>` に
揃えたので 1 対 1 で写せる。
"""


def keys(d):
    return [k for k, v in d.items()]


def joined(d):
    return [k + str(v) for k, v in d.items()]


def filt(pairs):
    return [a for a, b in pairs if b > 10]


def sset(d):
    return len({k for k, v in d.items()})


def with_enumerate(xs):
    return [str(i) + c for i, c in enumerate(xs)]

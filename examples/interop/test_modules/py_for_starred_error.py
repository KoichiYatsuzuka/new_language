"""py_for_starred_error.py — `*rest` を含む for ターゲット（明示エラー）。

残余を集める受け皿が Arrow の `targets` に無い。
"""


def g(rows):
    out = []
    for a, *rest in rows:
        out.append(a)
    return out

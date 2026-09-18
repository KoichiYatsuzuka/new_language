"""py_for_unpack.py — `for` のループ変数のアンパック（U1）。

Arrow の `Stmt::For.targets` は元から `Vec<String>` で**多ターゲットに対応済み**
だったので、変換器が単純名を並べるだけで通る。

⚠ `d.items()` は本項目と同時に Arrow へ追加した（それまで dict は `keys()` /
  `values()` しか持たず、Python で最頻出のこの形が使えなかった）。
"""


def from_pairs(pairs):
    out = []
    for k, v in pairs:
        out.append(str(k) + "->" + str(v))
    return out


def from_dict(d):
    total = 0
    for k, v in d.items():
        total = total + v
    return total


def with_enumerate(xs):
    out = []
    for i, x in enumerate(xs):
        out.append(str(i) + ":" + x)
    return out


def with_zip(a, b):
    out = []
    for p, q in zip(a, b):
        out.append(p + str(q))
    return out


def triple(rows):
    out = []
    for a, b, c in rows:
        out.append(a + b + c)
    return out

"""py_for_unpack.py — `for` のアンパックを使う関数（py_for_unpack.ar が CPython で動かす）。"""


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

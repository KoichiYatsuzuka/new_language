# py_int_element_reads.py — py_int_element_reads.ar から CPython で使うモジュール（pandas_import_plan.md の 2-12）。


def scores() -> list[int]:
    return [3, 1, 2]


def weights() -> list[float]:
    return [1, 2.5]          # PEP 484: float と注釈した所の int は正しい


def table() -> dict[str, int]:
    return {"a": 1}


def spoil(xs, x):
    """注釈を破る要素を後から入れる（Python は注釈を強制しない）。"""
    xs.append(x)


def spoil_value(d, k, v):
    d[k] = v

# py_import_in_function.py — 関数の中の import（フェーズ10 10-16・認めない）。
#
# 使う側: examples/interop/py_import_in_function_error.ar


def floor_of(x):
    import math
    return math.floor(x)

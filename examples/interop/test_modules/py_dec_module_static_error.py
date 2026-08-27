"""py_dec_module_static_error.py - モジュール直下の `@staticmethod` は明示エラーになる。

Python でも意味を持たない（呼べない descriptor になる）ので、黙って通さない。
"""


@staticmethod
def f(a):
    return a

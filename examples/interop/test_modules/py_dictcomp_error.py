"""py_dictcomp_error.py - 辞書内包は未対応（項目 17）。"""


def f(ks):
    return {k: k * 2 for k in ks}

"""py_fstring_spec_error.py - f-string の書式指定は Arrow に相当構文が無く明示エラー（項目 19）。"""


def f(x):
    return f"{x:.2f}"

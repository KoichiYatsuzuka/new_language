"""py_fstring_ascii_error.py - f-string の `!a`（ascii 変換）は相当する組込が無く明示エラー。"""


def f(x):
    return f"{x!a}"

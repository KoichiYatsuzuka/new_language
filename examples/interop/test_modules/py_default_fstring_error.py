"""py_default_fstring_error.py - f-string のデフォルト式も同じく明示エラー（項目 19 未実装）。"""


def g(msg=f"hi"):
    return msg

"""py_del_subscript_error.py — `del d[k]`（変換器が明示エラーにする形）。

`del <name>` は Arrow がスコープ退出で破棄するので**警告して無視**できるが、
`del d[k]` / `del o.a` は**意味のある削除**で、無視すると結果が変わる。
"""


def drop(d):
    del d["k"]
    return d

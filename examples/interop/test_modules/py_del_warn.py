"""py_del_warn.py — `del <name>` は警告つきで無視される（項目 14）／
例外まわりで**通り続ける**形の確認。

`del x` は束縛の削除で、Arrow はスコープ退出でどのみち破棄する。
⇒ 無視してよい。残る差は「削除後に読むと Python は `NameError`」だけ。
（`del d[k]` / `del o.a` は意味のある削除なので明示エラー。`py_stmt_errors_error.ar` 参照。）
"""


def del_name(n):
    tmp = n * 2
    del tmp
    return n


def plain_try(n):
    try:
        if n < 0:
            raise ValueError("neg")
        return "ok"
    except ValueError as e:
        return "caught"
    finally:
        pass


def bare_except():
    try:
        raise ValueError("x")
    except:
        return "bare"

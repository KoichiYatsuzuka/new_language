"""py_try_else_error.py — `try ... else:`（変換器が明示エラーにする形）。

`else` は「例外が起きなかったときだけ」走る節で `finally` とは違う。
黙って捨てると本体が丸ごと消える。
"""


def load(n):
    try:
        v = 10 // n
    except ZeroDivisionError:
        return "err"
    else:
        return v

"""py_walrus_while_error.py — `while` の条件での walrus（明示エラー）。

条件は**毎周回**評価される。持ち上げるとループの前に 1 回置かれるだけになり、
黙って別物になる。
"""


def g(it):
    while (n := len(it)) > 0:
        it = it[1:]
    return 0

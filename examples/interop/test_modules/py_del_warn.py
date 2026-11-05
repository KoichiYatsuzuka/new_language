"""py_del_warn.py — `del <name>` と例外の構文を使う関数（py_del_warn.ar が CPython で動かす）。"""


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

"""py_decorator_forward.py — デコレータの転送パターン（`*args` / `**kwargs` で包む）（py_decorator_forward.ar が CPython で動かす）。"""


def double(f):
    def wrapper(*args, **kwargs):
        return f(*args, **kwargs) * 2

    return wrapper


def log(f):
    def wrapper(*args, **kwargs):
        return "[" + str(f(*args, **kwargs)) + "]"

    return wrapper


@double
def triple(x):
    return x * 3


@log
@double
def stacked(x):
    return x + 1


def counter(start):
    # 関数値でなく int を捕捉する形（B3 で直っていた分）。
    def inc(step):
        return start + step

    return inc


def apply_twice(f, x):
    # 関数値を**引数で**受けて呼ぶ（入れ子なし・A7-a）。
    return f(f(x))

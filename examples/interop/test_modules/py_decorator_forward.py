"""py_decorator_forward.py — デコレータの転送パターン（U6 / フェーズ G）。

Python で最頻出の形。**4 つの機能が揃って初めて動く**:

1. `*args` / `**kwargs` を受ける（項目 6・7）
2. `f(*args, **kwargs)` で転送する（U4 / U5）
3. `mut` パラメータへ関数値を渡せる（A7-a）
4. 入れ子 `fn` が捕捉した関数値を**呼べる**（A7-b）
"""


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

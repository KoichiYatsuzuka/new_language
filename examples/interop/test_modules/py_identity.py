"""py_identity.py — `is` / `is not` を使う関数（py_identity.ar が CPython で動かす）。"""


class Box:
    def __init__(self, v):
        self.v = v


# --- 1. 最も多い用途: None 判定 ---
def is_none(x):
    return x is None


def is_not_none(x):
    return x is not None


def default_guard(x):
    if x is None:
        return "default"
    return x


# --- 2. 別名（同じオブジェクトを指す）---
def alias():
    a = [1, 2]
    b = a
    return a is b


def distinct():
    a = [1, 2]
    c = [1, 2]
    return a is c


def obj_alias():
    o = Box(1)
    p = o
    return o is p


# --- 3. True / False シングルトン ---
def is_true(f):
    return f is True


# --- 4. 組み合わせ ---
def combined(x, y):
    return x is None and y is not None


# --- 5. ★不変プリミティブは結果が違う（インターン依存。下の ⑥ 参照）---
def computed_str(a, b):
    x = a + b
    y = "hi"
    return x is y


def computed_int(a, b):
    x = a + b
    y = 1000
    return x is y

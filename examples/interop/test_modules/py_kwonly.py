"""py_kwonly.py — キーワード専用引数（bare `*`）・位置専用引数（`/`）を持つ関数（py_kwonly.ar が CPython で動かす）。"""


# --- 1. bare `*` の後ろに 1 個 ---
def f(a, *, b):
    return a * 10 + b


# --- 2. bare `*` の後ろに 2 個 ---
def g(a, *, b, c):
    return a + b + c


# --- 3. bare `*` だけ（位置引数なし） ---
def only_kw(*, b):
    return b + 1


# --- 4. 位置専用マーカ `/` と bare `*` の併用 ---
def both_markers(a, /, b, *, c):
    return a * 100 + b * 10 + c


# --- 5. `__init__` の bare `*` ---
class Point:
    def __init__(self, x, *, y):
        self.x = x
        self.y = y

    def total(self):
        return self.x + self.y

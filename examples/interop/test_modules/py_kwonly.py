"""py_kwonly.py - `import[py]` 変換器の bare `*`（キーワード専用引数）対応（項目 24）の検査用。

Python の `def f(a, *, b)` の `*` は「これより後ろはキーワードでしか渡せない」という区切り。
Arrow には対応する概念が無いので、変換器は **区切りを無視して通常引数に平坦化**する。
位置専用マーカ `/` も同じ扱い。
"""


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

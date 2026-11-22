"""py_classvar.py — クラス変数（全インスタンスで共有する可変の属性）を持つクラス（py_classvar.ar が CPython で動かす）。"""


class Counter:
    count = 0
    label = "c"
    items = []

    def __init__(self, v):
        self.v = v

    # --- クラス経由の読み書き ---
    def bump(self):
        Counter.count = Counter.count + 1
        return Counter.count

    def read(self):
        return Counter.count

    # --- `self` 経由の読み取り（クラス変数にフォールバックする） ---
    def via_self(self):
        return self.count

    # --- 可変オブジェクトのクラス変数は中身が共有される ---
    def push(self, x):
        Counter.items.append(x)
        return Counter.items

    # --- ★`self.x = ...` は Python では「インスタンス属性の新設」（下の ⑥ 参照） ---
    def shadow(self):
        self.count = 99
        return (self.count, Counter.count)


class Typed:
    """注釈つきクラス変数も同じ扱い（注釈は型として拾う）。"""

    n: int = 5
    s: str = "x"

    def get(self):
        return Typed.n

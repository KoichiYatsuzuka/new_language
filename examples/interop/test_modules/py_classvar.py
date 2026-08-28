"""py_classvar.py - `import[py]` 変換器のクラス変数対応（項目 5）の検査用モジュール。

Python のクラス属性は**可変・全インスタンス共有**なので、Arrow では
`FieldKind::StaticMut`（`static mut name: Type = default`）に対応する。
以前は `FieldKind::Const` にしていたため、`C.count = ...` が
`cannot assign to class variable (declared const)` で落ちていた。
"""


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

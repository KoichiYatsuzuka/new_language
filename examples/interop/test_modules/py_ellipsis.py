"""py_ellipsis.py - `import[py]` 変換器の `...`（Ellipsis）対応（項目 21）の検査用モジュール。

⚠ **文位置と値位置で扱いが違う**:
  - 文としての `...`（スタブ本体のプレースホルダ）→ Arrow の `pass`
  - 値としての `...`（`x = ...`）→ `None`（Arrow に `Ellipsis` 値が無いため。承認済みの仕様）
"""


# --- 1. 関数のスタブ本体 ---
def stub():
    ...


def stub_typed(x: int) -> int:
    ...


# --- 2. クラスの空本体 / メソッドのスタブ ---
class C:
    ...


class D:
    def m(self):
        ...


# --- 3. 制御構造の中のプレースホルダ ---
def guarded(x):
    if x > 0:
        ...
    else:
        return "neg"
    return "pos"


def loop_body(xs):
    for v in xs:
        ...
    return "done"


# --- 4. ★値位置の `...` は None になる（ここだけ CPython と表示が違う） ---
def value_pos():
    x = ...
    return x

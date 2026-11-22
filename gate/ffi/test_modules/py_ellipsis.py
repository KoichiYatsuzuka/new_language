"""py_ellipsis.py - `...`（Ellipsis）を文と値の位置で使うモジュール（py_ellipsis.ar が CPython で動かす）。"""


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


# --- 4. ★値位置の `...` は Ellipsis（CPython と同じ・タスク 6-11） ---
def value_pos():
    x = ...
    return x

"""py_decorators.py - `import[py]` 変換器のデコレータ対応（項目 20）の検査用モジュール。

Arrow 側は `@decorator` を定義時に**逆順**で適用する。
`@staticmethod` / `@classmethod` は Arrow に同名の組込関数が無いので、
変換器が `static fn` / `class_method fn` のフラグに振り替える。
"""

# `@abstractmethod` を素の Python としても有効にするための import。
# ⚠ 変換器は Python 内 import を現状**黙って捨てる**（項目 27 未実装）。
# ここでは `@abstractmethod` を変換器が先に横取りするため、捨てられても問題にならない。
from abc import abstractmethod


# --- 1. 関数デコレータ（元の関数をそのまま返す） ---
def banner(f):
    print("registered")
    return f


@banner
def hello(name):
    return "hello " + name


# --- 2. スタックされたデコレータ（下から順に適用） ---
def tag_a(f):
    print("A")
    return f


def tag_b(f):
    print("B")
    return f


@tag_a
@tag_b
def stacked():
    return "stacked"


# --- 3. 関数を**別の関数に差し替える**デコレータ ---
def always42(f):
    def fixed(n):
        return 42

    return fixed


@always42
def twice(n):
    return n * 2


# --- 4. クラスデコレータ ---
def mark(cls):
    print("marked")
    return cls


@mark
class Box:
    def __init__(self, v):
        self.v = v

    def get(self):
        return self.v

    # --- 5. `@staticmethod` -> Arrow の `static fn`（クラス経由で呼ぶ） ---
    @staticmethod
    def bump(a):
        return a + 1

    # --- 6. `@classmethod` -> Arrow の `class_method fn`（第 1 引数にクラス） ---
    @classmethod
    def of(cls, v):
        return cls(v)

    # --- 7. `@abstractmethod` は Arrow の抽象フラグに振り替え（本体はそのまま） ---
    @abstractmethod
    def described(self):
        return "box"

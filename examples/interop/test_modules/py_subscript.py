"""py_subscript.py - `import[py]` 変換器の添字/キー代入対応（項目 3）の検査用モジュール。

Arrow は添字代入を「代入先の**式**を `target` に置く `Stmt::AttrAssign`」で表す
（属性代入 `o.a = v` と同じノード）。複合代入は `Stmt::AttrCompoundAssign`。
"""


# --- 1. dict のキー代入・読んで書き戻し ---
def dict_set():
    d = {"a": 1}
    d["b"] = 5
    d["a"] = d["a"] + 10
    return d


# --- 2. list の添字代入 ---
def list_set():
    xs = [1, 2, 3]
    xs[0] = 99
    return xs


# --- 3. 複合代入 ---
def aug_list():
    xs = [1, 2, 3]
    xs[1] += 7
    xs[2] *= 3
    return xs


def aug_dict():
    d = {"n": 10}
    d["n"] -= 4
    return d


# --- 4. 入れ子（target が入れ子の Subscript になるだけ） ---
def nested():
    box = {"k": [1, 2]}
    box["k"][0] = 42
    box["k"][1] += 8
    return box


# --- 5. 属性 + 添字の組み合わせ ---
class Holder:
    def __init__(self):
        self.data = [0, 0]

    def bump(self, i):
        self.data[i] += 1
        return self.data


def attr_and_index(o):
    o.data[0] = 7
    return o.data


# --- 6. ループの中で dict を組み立てる（典型的な用途） ---
def in_loop(n):
    counts = {}
    for i in range(n):
        counts[i] = i * i
    return counts

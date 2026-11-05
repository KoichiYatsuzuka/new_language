"""py_defaults.py — 既定値のある引数を持つ関数（py_defaults.ar が CPython で動かす）。"""


# --- 1. 位置引数のデフォルト 1 個 ---
def greet(name, greeting="hi"):
    return greeting + " " + name


# --- 2. デフォルト複数（後ろから埋まる） ---
def add(a, b=10, c=100):
    return a + b + c


# --- 3. キーワード専用引数のデフォルト ---
def kw(a, *, b=3, c=4):
    return a * 100 + b * 10 + c


# --- 4. None デフォルト ---
def nonespec(x=None):
    return x


# --- 5. タプル・リテラルのデフォルト（読むだけなら Python と同じ） ---
def pair(t=(1, 2)):
    return t[0] + t[1]


# --- 6. `__init__` のデフォルト ---
class Box:
    def __init__(self, v=7, label="box"):
        self.v = v
        self.label = label

    def show(self):
        return self.label + "=" + str(self.v)


# --- 7. ★可変デフォルトの罠: ここだけ Python と結果が違う ---
#     Python は `def` 実行時に 1 個だけ作ったリストを呼び出し間で共有するので 1, 2, 3 …
#     Arrow は呼び出しごとにデフォルト式を評価するので毎回 1。
def mutdef(xs=[]):
    xs.append(1)
    return len(xs)

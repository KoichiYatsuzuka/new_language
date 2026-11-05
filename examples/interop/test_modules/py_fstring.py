"""py_fstring.py — f-string を使う関数（py_fstring.ar が CPython で動かす）。"""


class Box:
    def __init__(self, v):
        self.v = v


# --- 1. 基本形 ---
def simple(name):
    return f"hi {name}"


def multi(name, n):
    return f"hi {name}, n={n}!"


def only_expr(n):
    return f"{n}"


def empty():
    return f""


def literal_only():
    return f"plain"


def adjacent(a, b):
    return f"{a}{b}"


# --- 2. `{{` / `}}` はリテラルの波括弧 ---
def braces(n):
    return f"{{literal}} {n}"


# --- 3. 埋め込みは任意の式でよい ---
def expr_inside(a, b):
    return f"sum={a + b}"


def call_inside(xs):
    return f"len={len(xs)}"


def attr_inside(o):
    return f"v={o.v}"


def index_inside(d):
    return f"x={d['x']}"


# --- 4. 変換フラグ `!s` / `!r`（Arrow の str() / repr() に写せる） ---
def conv_str(s):
    return f"{s!s}"


def conv_repr(s):
    return f"{s!r}"


# --- 5. f-string の結果をさらに f-string に埋める ---
def nested_fstring(n):
    inner = f"[{n}]"
    return f"outer {inner}"

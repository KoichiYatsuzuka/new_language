# py_open.py — Python のコードの open(..)（CPython の open）。
#
# 使う側: examples/interop/py_open.ar


def save(path, lines):
    with open(path, "w", encoding="utf-8") as f:
        for line in lines:
            f.write(line + "\n")


def load(path):
    # with の中の return
    with open(path, encoding="utf-8") as f:
        return f.read()

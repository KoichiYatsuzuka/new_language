# py_open.py — Python のコードの open(..)（python_builtins_plan.md のタスク 5-2）。
#
# 使う側: examples/interop/py_open.ar


def save(path, lines):
    with open(path, "w", encoding="utf-8") as f:
        for line in lines:
            f.write(line + "\n")


def load(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    return text

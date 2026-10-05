# py_open.py — Python のコードの open(..)（python_builtins_plan.md のタスク 5-2）。
#
# 使う側: examples/interop/py_open.ar


def save(path, lines):
    with open(path, "w", encoding="utf-8") as f:
        for line in lines:
            f.write(line + "\n")


def load(path):
    # with の中の return（変換先は block: 文の中の return・タスク 5-3 で通るようにした）。
    with open(path, encoding="utf-8") as f:
        return f.read()

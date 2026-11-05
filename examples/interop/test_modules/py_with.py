"""py_with.py — `with` 文（ファイル・`__enter__` / `__exit__` を持つクラス）を使うモジュール（py_with.ar が CPython で動かす）。"""


def write_hello(p):
    with open(p, "w") as f:
        f.write("hello")
    return "written"


def write_two(p, q):
    # 複数アイテム（宣言順に評価される）
    with open(p, "w") as a, open(q, "w") as b:
        a.write("A")
        b.write("B")
    return "written2"


def no_binding(p):
    # `as` なし
    with open(p, "w"):
        pass
    return "no-as"


class Lock:
    def __init__(self):
        self.log = []

    def __enter__(self):
        self.log.append("enter")
        return "token"

    def __exit__(self, kind, value, tb):
        self.log.append("exit:" + (kind.__name__ if kind else "ok"))
        return False


def with_hooks():
    lock = Lock()
    with lock as t:
        lock.log.append(t)
    try:
        with lock:
            raise ValueError("boom")
    except ValueError:
        pass
    return lock.log

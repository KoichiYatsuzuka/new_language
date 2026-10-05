# py_except_tuple.py — Python の except (A, B):（python_builtins_plan.md のタスク 4-4）。
#
# 使う側: examples/interop/py_except_tuple.ar

import test_modules.py_exception_hierarchy as h


def parse(s):
    try:
        return int(s)
    except (ValueError, TypeError) as e:
        return -1


def classify(kind):
    try:
        if kind == 1:
            raise KeyError("k")
        if kind == 2:
            raise IndexError("i")
        if kind == 3:
            h.fail()
        raise RuntimeError("r")
    except (KeyError, IndexError, h.ConfigError) as e:
        return type(e).__name__ + " in the tuple"
    except Exception as e:
        return type(e).__name__ + " fell through"

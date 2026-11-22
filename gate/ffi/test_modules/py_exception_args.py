# py_exception_args.py — Python の例外の args / str(e) / super().__init__（python_builtins_plan.md のタスク 4-2）。
#
# 使う側: examples/interop/py_exception_args.ar


class RangeError(Exception):
    def __init__(self, low, high):
        self.low = low
        super().__init__(f"{low}-{high}")


class Missing(LookupError):
    pass


class Coded(Exception):
    def __init__(self, code):
        self.code = code


def make_all():
    return [RangeError(1, 9), Missing("name", 2), Coded(7)]


def describe(e):
    return f"{type(e).__name__}: str={str(e)} args={e.args}"


def describe_all():
    return [describe(e) for e in make_all()]


def fail():
    raise RangeError(3, 4)

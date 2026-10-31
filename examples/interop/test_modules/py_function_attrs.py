# py_function_attrs.py — 関数の値の __name__ / __qualname__ / __doc__（python_builtins_plan.md のタスク 1-14）
# CPython 3.12 と同じ出力になることを py_function_attrs.ar で確かめる。

calls = []


def logged(func):
    """Record each call under the function's name."""
    def wrapper(*args):
        calls.append(func.__name__)
        return func(*args)
    return wrapper


@logged
def add(a, b):
    """Add two numbers."""
    return a + b


def make_counter():
    def step(n):
        return n + 1
    return step


def make_adder():
    # 関数の中の関数も Python の関数（既定値・キーワード引数の束縛）。
    def adder(a, b=2, **kw):
        return a + b + sum(kw.values())
    return adder


class Temperature:
    def __init__(self, c):
        self._c = c

    @property
    def celsius(self):
        """Degrees Celsius."""
        return self._c

    @celsius.setter
    def celsius(self, v):
        self._c = v

    def describe(self):
        def unit():
            return "C"
        return str(self._c) + unit.__qualname__


def run():
    print(add(1, 2), calls)
    print(add.__name__, add.__qualname__)
    print(logged.__name__, logged.__doc__)
    step = make_counter()
    print(step.__name__, step.__qualname__, step.__doc__)
    adder = make_adder()
    print(adder(1), adder(1, b=5), adder(1, 2, c=10), adder.__qualname__)
    square = lambda x: x * x
    print(square.__name__, square.__qualname__)
    t = Temperature(20)
    print(Temperature.describe.__qualname__, t.describe.__name__, t.describe())
    p = Temperature.celsius
    print(p.fget.__name__, p.fget.__qualname__, p.fget.__doc__)
    print(p.fset.__name__, p.fset.__qualname__)
    print(run.__name__, run.__qualname__, run.__doc__)

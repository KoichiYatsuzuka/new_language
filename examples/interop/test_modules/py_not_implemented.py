"""py_not_implemented.py - NotImplemented と演算子の手順（python_builtins_plan.md のタスク 6-12）。"""


class Money:
    def __init__(self, cents):
        self.cents = cents

    def __add__(self, other):
        if isinstance(other, Money):
            return Money(self.cents + other.cents)
        if isinstance(other, int):
            return Money(self.cents + other)
        return NotImplemented

    def __radd__(self, other):
        # sum() は 0 + m から始まる
        return self.__add__(other)

    def __eq__(self, other):
        if not isinstance(other, Money):
            return NotImplemented
        return self.cents == other.cents

    def __lt__(self, other):
        if not isinstance(other, Money):
            return NotImplemented
        return self.cents < other.cents

    def __repr__(self):
        return f"Money({self.cents})"


class Meters:
    def __init__(self, v):
        self.v = v

    def __mul__(self, k):
        return Meters(self.v * k) if isinstance(k, (int, float)) else NotImplemented

    def __rmul__(self, k):
        return self.__mul__(k)

    def __gt__(self, other):
        return isinstance(other, (int, float)) and self.v > other

    def __repr__(self):
        return f"Meters({self.v})"


class Plain:
    pass


class Base:
    def __add__(self, other):
        return "Base.__add__"


class Derived(Base):
    def __radd__(self, other):
        return "Derived.__radd__"


def attempt(label, f):
    try:
        print(label, f())
    except TypeError as e:
        print(label, "TypeError:", e)


def run():
    a, b = Money(150), Money(250)
    # 反対側の __radd__ / __rmul__
    print(a + b, a + 5, 5 + a, sum([a, b]), 3 * Meters(2), Meters(2) * 3)
    # == は反対側の __eq__ を試し、最後は同一性。!= は __eq__ の反対
    print(a == Money(150), a != Money(150), a == 150, 150 == a, a != "x", a == a)
    # 大小比較は反対側（< の反対は >）
    print(a < b, 3 < Meters(5), Meters(5) > 3)
    # 両方が NotImplemented / 演算子メソッドが無い → TypeError（CPython と同じ文言）
    attempt("a + 'x':", lambda: a + "x")
    attempt("'x' + a:", lambda: "x" + a)
    attempt("a < 1:", lambda: a < 1)
    attempt("1 >= a:", lambda: 1 >= a)
    attempt("Plain() * 2:", lambda: Plain() * 2)
    attempt("2 ** Plain():", lambda: 2 ** Plain())
    attempt("Plain() // Plain():", lambda: Plain() // Plain())
    # 右が左の派生で __radd__ を上書きしていれば右が先
    print(Base() + Derived(), Derived() + Base())
    # NotImplemented は値としても使える
    r = a.__add__("x")
    print(r is NotImplemented, r, repr(NotImplemented), type(NotImplemented).__name__)
    print(type(NotImplemented)() is NotImplemented, isinstance(NotImplemented, object), NotImplemented == NotImplemented)

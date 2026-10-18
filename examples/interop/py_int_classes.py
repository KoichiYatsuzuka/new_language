# py_int_classes.py - Python classes used from Arrow through import[py-int] (ffi_value_exchange_plan.md 7-27).
# Only annotated members get types on the Arrow side; the constructor gives the class's own type.


class Counter:
    label: str = "counter"           # class attribute with an annotation -> str

    def __init__(self, start: int, step=1) -> None:
        self.value: int = start      # annotated -> int
        self.step = step             # not annotated -> Any
        self.history: list[int] = []

    def bump(self) -> int:
        self.value += self.step
        self.history.append(self.value)
        return self.value

    def describe(self):              # not annotated -> Any
        return f"{self.label}={self.value}"

    @property
    def count(self) -> int:
        return len(self.history)


class LoudCounter(Counter):          # a base class in the same file: its members are known too
    def shout(self) -> str:
        return self.describe().upper()


def make_counter(start: int) -> "Counter":
    return Counter(start)


def untyped(x):                      # not annotated -> Any
    return x

"""py_property_error.py - setter の無い property への代入（python_builtins_plan.md のタスク 6-1）。"""


class ReadOnly:
    @property
    def value(self):
        return 42


def run():
    r = ReadOnly()
    print(r.value)
    r.value = 1
    print("unreachable")

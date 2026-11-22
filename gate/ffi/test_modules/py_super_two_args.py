"""py_super_two_args.py - `super(C, self)` の 2 引数の形（python_builtins_plan.md のタスク 6-2）。"""


class Shape(object):
    def __init__(self, name):
        super(Shape, self).__init__()
        self.name = name

    def describe(self):
        return "shape " + self.name


class Square(Shape):
    def __init__(self, side):
        super(Square, self).__init__("square")
        self.side = side

    def describe(self):
        return super(Square, self).describe() + " of side " + str(self.side)


class Labeled(Square):
    def __init__(self, side, label):
        super().__init__(side)
        self.label = label

    def describe(self):
        return self.label + ": " + super(Labeled, self).describe()


def run():
    s = Square(3)
    print(s.name, s.side)
    print(s.describe())
    print(Labeled(2, "tile").describe())

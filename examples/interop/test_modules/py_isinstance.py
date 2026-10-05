# py_isinstance.ar から import する Python のモジュール（isinstance を Python の中で使う）。


class Animal:
    pass


class Dog(Animal):
    pass


class Puppy(Dog):
    pass


def kind(x):
    if isinstance(x, bool):
        return "bool"
    if isinstance(x, int):
        return "int"
    if isinstance(x, (list, tuple)):
        return "sequence"
    if isinstance(x, dict):
        return "dict"
    if isinstance(x, Dog):
        return "a Dog"
    if isinstance(x, Animal):
        return "an Animal"
    return "other"


def kinds():
    return [kind(True), kind(3), kind([1]), kind((1,)), kind({}), kind(Puppy()), kind(Animal()), kind("s")]


def by_variable(x, cls):
    return isinstance(x, cls)

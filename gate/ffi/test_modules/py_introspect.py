# py_introspect.ar から import する Python のモジュール。型の判定と属性の名前による読み書きを使う。


class Animal:
    sound = "..."

    def __init__(self, name):
        self.name = name

    def speak(self):
        return self.name + " says " + self.sound


class Dog(Animal):
    sound = "woof"


class Puppy(Dog):
    pass


def describe(x):
    if isinstance(x, bool):
        return "bool"
    if isinstance(x, int):
        return "int"
    if isinstance(x, (list, tuple)):
        return "sequence of " + str(len(x))
    if isinstance(x, dict):
        return "dict"
    if isinstance(x, Animal):
        return type(x).__name__ + " (an Animal)"
    return "other: " + type(x).__name__


def kinds():
    return [describe(True), describe(3), describe([1, 2]), describe((1,)), describe({}),
            describe(Puppy("rex")), describe("s")]


def by_variable(x):
    cls = Animal
    return isinstance(x, cls)


def subclasses():
    return [issubclass(Puppy, Animal), issubclass(Dog, Puppy), issubclass(bool, int),
            issubclass(Dog, (int, Animal))]


def callables():
    d = Dog("a")
    return [callable(describe), callable(Dog), callable(3), callable(d.speak), callable(len)]


def attributes():
    d = Dog("rex")
    before = getattr(d, "name")
    missing = getattr(d, "color", "brown")
    has = [hasattr(d, "speak"), hasattr(d, "fly")]
    setattr(d, "name", "max")
    return [before, missing, has, d.name, d.speak()]


def types():
    d = Dog("rex")
    again = type(d)("copy")
    return [type(d) is Dog, type(3) is int, type(d) is Animal, again.speak(), type(True).__name__]

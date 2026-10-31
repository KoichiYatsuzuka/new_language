# py_module_types.ar から import する Python のモジュール（クラスの継承の連なり）。


class Animal:
    def kind(self):
        return "animal"


class Dog(Animal):
    def kind(self):
        return "dog"


class Puppy(Dog):
    def kind(self):
        return "puppy"

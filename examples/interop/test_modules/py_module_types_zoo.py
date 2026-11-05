# py_module_types.ar から import する Python のモジュール（クラスの継承の連なり）。


class Animal:
    def kind(self) -> str:
        return "animal"


class Dog(Animal):
    def kind(self) -> str:
        return "dog"


class Puppy(Dog):
    def kind(self) -> str:
        return "puppy"

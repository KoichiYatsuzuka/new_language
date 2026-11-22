def add(a: int, b: int) -> int:
    return a + b

def subtract(a: int, b: int) -> int:
    return a - b

def multiply(a: int, b: int) -> int:
    return a * b

def divide(a: float, b: float) -> float:
    if b == 0:
        return 0.0
    return a / b

class Calculator:
    def __init__(self, name: str):
        self.name = name
        self.last = 0

    def compute(self, op: str, a: int, b: int) -> int:
        if op == "add":
            result = add(a, b)
        elif op == "sub":
            result = subtract(a, b)
        elif op == "mul":
            result = multiply(a, b)
        else:
            result = 0
        self.last = result
        return result

    def last_result(self) -> int:
        return self.last

PI = 3

def add_flexible(a: int, b: int) -> int:
    return a + b

def func(**kwargs):
    for key in kwargs.keys():
        print(kwargs[key])

class Container:
    """Python オブジェクトとして残る（tl の Tuple/List に変換されない）コンテナ。
    __getitem__ / __setitem__ の PyObject 経由テスト用。"""
    def __init__(self, data: list):
        self.data = list(data)

    def __getitem__(self, key: int):
        return self.data[key]

    def __setitem__(self, key: int, value):
        self.data[key] = value

    def __iter__(self):
        return iter(self.data)

    def __len__(self):
        return len(self.data)

    def __mul__(self, n: int) -> "Container":
        return Container(self.data * n)

    def __rmul__(self, n: int) -> "Container":
        return Container(self.data * n)

    def __add__(self, other: "Container") -> "Container":
        return Container(self.data + other.data)

def make_container(data: list) -> "Container":
    return Container(data)

def sum_dict(d: dict) -> int:
    return sum(d.values())

def first_of_tuple(t: tuple):
    return t[0]

def identity_tuple(t: tuple) -> tuple:
    return t

if __name__ == "__main__":
    c = Calculator("test")
    print(c.compute("add", 1, 2))

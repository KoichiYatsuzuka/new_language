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

if __name__ == "__main__":
    c = Calculator("test")
    print(c.compute("add", 1, 2))

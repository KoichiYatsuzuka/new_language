import copy
from typing import Callable, Generator

print('=== basic functions ===')
def add(a: int, b: int) -> int:
    return (a + b)

def greet(name: str) -> str:
    return ('Hello, ' + name)

def factorial(n: int) -> int:
    result = 1
    while (n > 1):
        result *= n
        n -= 1
    return result

print(add(3, 4))
print(greet('world'))
print(factorial(6))
print('=== typed function signatures ===')
def apply(f: Callable, x: int) -> int:
    return f(x)

def double(n: int) -> int:
    return (n * 2)

print(apply(double, 5))
def make_namer() -> Callable:
    def inner(value: int) -> int:
        return (value * 3)

    return inner

namer = make_namer()
print(namer(value=7))
print(namer(7))
def make_const() -> Callable:
    def always42() -> int:
        return 42

    return always42

f0 = make_const()
print(f0())
def call_any(f: Callable) -> None:
    f()

def say_hi() -> None:
    print('hi!')

call_any(say_hi)
print('=== default parameters ===')
def greet_default(name: str = 'world') -> str:
    return (('Hello, ' + name) + '!')

print(greet_default())
print(greet_default('Alice'))
def add_defaults(a: int = 1, b: int = 2) -> int:
    return (a + b)

print(add_defaults())
print(add_defaults(10))
print(add_defaults(10, 20))
def repeat(s: str, n: int = 3) -> str:
    result = ''
    for i in range(n):
        result = (result + s)
    return result

print(repeat('ab'))
print(repeat('ab', 2))
def scale(x: int = 0, y: int = 0) -> int:
    return ((x * 100) + y)

print(scale())
print(scale(1))
print(scale(y=5))
print(scale(3, 4))
class Counter:
    count: int = 0
    def increment(self, by: int = 1) -> None:
        self.count = (self.count + by)


ctr = Counter()
ctr.increment()
ctr.increment()
ctr.increment(5)
print(ctr.count)
print('=== closures: let capture ===')
def make_greeter(name: str) -> Callable:
    def inner_greet() -> str:
        return ('Hello, ' + name)

    return inner_greet

greet_alice = make_greeter('Alice')
greet_bob = make_greeter('Bob')
print(greet_alice())
print(greet_bob())
print('=== closures: mut capture ===')
def make_counter() -> Callable:
    count = 0
    def inc() -> int:
        count += 1
        return count

    return inc

ca = make_counter()
cb = make_counter()
print(ca())
print(ca())
print(cb())
print(ca())
print('=== closures: static mut ===')
def make_global_counter() -> Callable:
    count = 0  # static mut
    def inc() -> int:
        count += 1
        return count

    return inc

sc1 = make_global_counter()
sc2 = make_global_counter()
print(sc1())
print(sc2())
print(sc1())
print('=== generators ===')
def range_step(start: int, stop: int, step: int) -> Generator[int, None, None]:
    i = start
    while (i < stop):
        yield i
        i += step

g = range_step(0, 10, 3)
print(g.next())
print(g.next())
print(g.next())
print(g.next())
total = 0
for v in range_step(1, 6, 1):
    total += v
print('sum 1-5:', copy.deepcopy(total))
print('=== decorators: basic wrapper ===')
def logged(f: Callable) -> Callable:
    def wrapper() -> None:
        print('>> calling')
        f()
        print('<< done')

    return wrapper

@logged
def hello() -> None:
    print('hello!')

hello()
def timed(f: Callable) -> Callable:
    def wrapper() -> None:
        print('[timed]')
        f()

    return wrapper

@timed
@logged
def goodbye() -> None:
    print('goodbye!')

goodbye()
print('=== decorators: call counter (closure + decorator) ===')
def count_calls(f: Callable) -> Callable:
    calls = 0
    def wrapper() -> None:
        calls += 1
        print('call #', calls)
        f()

    return wrapper

@count_calls
def compute() -> None:
    print('computing...')

compute()
compute()
compute()
print('=== decorators: parameterised (decorator factory) ===')
def repeat_deco(n: int) -> Callable:
    def decorator(f: Callable) -> Callable:
        def wrapper() -> None:
            i = 0
            while (i < n):
                f()
                i += 1

        return wrapper

    return decorator

@repeat_deco(3)
def ping() -> None:
    print('ping')

ping()

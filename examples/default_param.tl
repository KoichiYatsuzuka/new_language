# default_param.tl — default parameter behavior examples

# Basic default parameter
fn greet(let name: str = "world") -> str:
    return "Hello, " + name + "!"

print(greet())         # Hello, world!
print(greet("Alice"))  # Hello, Alice!

# Multiple default parameters
fn add(let a: int = 1, let b: int = 2) -> int:
    return a + b

print(add())       # 3
print(add(10))     # 12
print(add(10, 20)) # 30

# Mixed: required then optional
fn repeat(let s: str, let n: int = 3) -> str:
    mut result = ""
    for i in range(n):
        result = result + s
    return result

print(repeat("ab"))     # ababab
print(repeat("ab", 2))  # abab

# Keyword argument filling a gap while using a default for the other
fn scale(let x: int = 0, let y: int = 0) -> int:
    return x * 100 + y

print(scale())          # 0
print(scale(1))         # 100
print(scale(y=5))       # 5
print(scale(3, 4))      # 304

# Default in method
class Counter:
    mut count: int = 0

    fn increment(mut self, let by: int = 1) -> None:
        self.count = self.count + by

mut c = Counter()
c.increment()
c.increment()
c.increment(5)
print(c.count)  # 7

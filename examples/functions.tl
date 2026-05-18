# functions.tl — basic functions, typed signatures, default params,
#                closures, generators, and decorators

# ============================================================
# 1. Basic functions
# ============================================================
print("=== basic functions ===")

fn add(let a: int, let b: int) -> int:
    return a + b

fn greet(let name: str) -> str:
    return "Hello, " + name

fn factorial(mut n: int) -> int:
    mut result = 1
    while n > 1:
        result *= n
        n -= 1
    return result

print(add(3, 4))          # 7
print(greet("world"))     # Hello, world
print(factorial(6))       # 720

# ============================================================
# 2. Typed function signatures
# ============================================================
print("=== typed function signatures ===")

# function[param_types]->ret: positional typed params
fn apply(let f: function[let int]->int, let x: int) -> int:
    return f(x)

fn double(let n: int) -> int:
    return n * 2

print(apply(double, 5))   # 10

# function{named_param:type}->ret: named typed params
fn make_namer() -> function{let value:int}->int:
    fn inner(let value: int) -> int:
        return value * 3
    return inner

let namer = make_namer()
print(namer(value=7))   # 21
print(namer(7))          # 21 (positional also works)

# function[]->ret: zero-parameter function
fn make_const() -> function[]->int:
    fn always42() -> int:
        return 42
    return always42

let f0 = make_const()
print(f0())   # 42

# bare function: any signature (no static checking on args)
fn call_any(let f: function) -> None:
    f()

fn say_hi() -> None:
    print("hi!")

call_any(say_hi)   # hi!

# ============================================================
# 3. Default parameters
# ============================================================
print("=== default parameters ===")

fn greet_default(let name: str = "world") -> str:
    return "Hello, " + name + "!"

print(greet_default())         # Hello, world!
print(greet_default("Alice"))  # Hello, Alice!

# Multiple default parameters
fn add_defaults(let a: int = 1, let b: int = 2) -> int:
    return a + b

print(add_defaults())       # 3
print(add_defaults(10))     # 12
print(add_defaults(10, 20)) # 30

# Mixed: required then optional
fn repeat(let s: str, let n: int = 3) -> str:
    mut result = ""
    for i in range(n):
        result = result + s
    return result

print(repeat("ab"))     # ababab
print(repeat("ab", 2))  # abab

# Keyword argument with default
fn scale(let x: int = 0, let y: int = 0) -> int:
    return x * 100 + y

print(scale())      # 0
print(scale(1))     # 100
print(scale(y=5))   # 5
print(scale(3, 4))  # 304

# Default in method
class Counter:
    mut count: int = 0

    fn increment(mut self, let by: int = 1) -> None:
        self.count = self.count + by

mut ctr = Counter()
ctr.increment()
ctr.increment()
ctr.increment(5)
print(ctr.count)  # 7

# ============================================================
# 4. Closures — let capture (deep copy at definition time)
# ============================================================
print("=== closures: let capture ===")

fn make_greeter(let name: str) -> function[]->str:
    fn inner_greet() -> str:
        return "Hello, " + name
    return inner_greet

let greet_alice = make_greeter("Alice")
let greet_bob   = make_greeter("Bob")
print(greet_alice())   # Hello, Alice
print(greet_bob())     # Hello, Bob

# ============================================================
# 5. Closures — mut capture (shared cell, independent per call)
# ============================================================
print("=== closures: mut capture ===")

fn make_counter() -> function[]->int:
    mut count = 0
    fn inc() -> int:
        count += 1
        return count
    return inc

let ca = make_counter()
let cb = make_counter()   # independent environment
print(ca())   # 1
print(ca())   # 2
print(cb())   # 1 (cb is independent from ca)
print(ca())   # 3

# ============================================================
# 6. Closures — static mut (shared across ALL calls)
# ============================================================
print("=== closures: static mut ===")

fn make_global_counter() -> function[]->int:
    static mut count = 0
    fn inc() -> int:
        count += 1
        return count
    return inc

let sc1 = make_global_counter()
let sc2 = make_global_counter()   # shares same count cell
print(sc1())   # 1
print(sc2())   # 2 (sc1 and sc2 share the static cell)
print(sc1())   # 3

# ============================================================
# 7. Generators (gen + yield)
# ============================================================
print("=== generators ===")

gen range_step(start: int, stop: int, step: int) -> int:
    mut i = start
    while i < stop:
        yield i
        i += step

let g = range_step(0, 10, 3)
print(g.next())   # 0
print(g.next())   # 3
print(g.next())   # 6
print(g.next())   # 9

mut total = 0
for v in range_step(1, 6, 1):
    total += v
print("sum 1-5:", total)   # 15

# ============================================================
# 8. Decorators
# ============================================================
print("=== decorators: basic wrapper ===")

fn logged(let f: function) -> function:
    fn wrapper() -> None:
        print(">> calling")
        f()
        print("<< done")
    return wrapper

@logged
fn hello() -> None:
    print("hello!")

hello()
# >> calling
# hello!
# << done

# Stacked decorators (applied bottom-up: logged first, then timed)
fn timed(let f: function) -> function:
    fn wrapper() -> None:
        print("[timed]")
        f()
    return wrapper

@timed
@logged
fn goodbye() -> None:
    print("goodbye!")

goodbye()
# [timed]
# >> calling
# goodbye!
# << done

print("=== decorators: call counter (closure + decorator) ===")

# Decorator that uses a mut closure to count invocations
fn count_calls(let f: function) -> function:
    mut calls = 0
    fn wrapper() -> None:
        calls += 1
        print("call #", calls)
        f()
    return wrapper

@count_calls
fn compute() -> None:
    print("computing...")

compute()   # call #1 / computing...
compute()   # call #2 / computing...
compute()   # call #3 / computing...

print("=== decorators: parameterised (decorator factory) ===")

# A decorator factory: repeat(n) returns a decorator that runs f n times
fn repeat_deco(let n: int) -> function:
    fn decorator(let f: function) -> function:
        fn wrapper() -> None:
            mut i = 0
            while i < n:
                f()
                i += 1
        return wrapper
    return decorator

@repeat_deco(3)
fn ping() -> None:
    print("ping")

ping()
# ping
# ping
# ping

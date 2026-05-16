# function_closure.tl — functions, closures, generators, and decorators

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
# 3. Closures — let capture (deep copy at definition time)
# ============================================================
print("=== closures: let capture ===")

fn make_greeter(let name: str) -> function[]->str:
    fn greet() -> str:
        return "Hello, " + name
    return greet

let greet_alice = make_greeter("Alice")
let greet_bob   = make_greeter("Bob")
print(greet_alice())   # Hello, Alice
print(greet_bob())     # Hello, Bob

# ============================================================
# 4. Closures — mut capture (shared cell, independent per call)
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
# 5. Closures — static mut (shared across ALL calls)
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
# 6. Generators (gen + yield)
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

# Generator as iterator in for loop
mut total = 0
for v in range_step(1, 6, 1):
    total += v
print("sum 1-5:", total)   # 15

# ============================================================
# 7. Decorators
# ============================================================
print("=== decorators ===")

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

# Stacked decorators (applied bottom-up)
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

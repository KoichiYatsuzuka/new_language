# function type examples

# --- bare function type (any params, any return) ---
fn apply(let f: function, let x: int) -> int:
    return f(x)

fn double(let x: int) -> int:
    return x * 2

let r1 = apply(double, 5)  # 10

# --- function[params]->ret: positional typed params ---
fn make_adder() -> function[let int]->int:
    fn adder(let n: int) -> int:
        return n + 1
    return adder

mut x = 1
x = make_adder()(3)  # OK: chained call returns int

let y = make_adder()   # y: function[let int]->int
let z = y(3)           # OK: z is int
# let w = y("str")    # Would be StaticTypeError (commented out)

# --- function{named params}->ret ---
fn make_namer() -> function{let value:int}->int:
    fn inner(let value: int) -> int:
        return value * 2
    return inner

mut a = 1
a = make_namer()(value = 10)   # OK: positional also works
let b = make_namer()(value = z)  # OK: z is int (immutable)
let c = make_namer()(10)       # OK: positional

# --- function[]->ret: zero parameters ---
fn make_constant() -> function[]->int:
    fn const_fn() -> int:
        return 42
    return const_fn

let fn0 = make_constant()
let r2 = fn0()  # 42

# --- is type guard for function ---
fn some_fn(let n: int) -> int:
    return n

let is_fn = some_fn is function  # True

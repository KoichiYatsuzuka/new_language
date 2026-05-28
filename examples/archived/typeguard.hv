# typeguard.tl — Demonstrates `is` / `is not` type guard behaviour

# --- Basic `is` guards with primitives ---
fn describe(x: Any) -> str:
    if x is int:
        return "integer"
    if x is str:
        return "string"
    if x is float:
        return "float"
    if x is bool:
        return "bool"
    return "other"

print(describe(42))      # integer
print(describe("hello")) # string
print(describe(3.14))    # float
print(describe(True))    # bool

# --- Class type guard ---
class Cat:
    let name: str

class Dog:
    let name: str
    let breed: str

fn pet_kind(p: Any) -> str:
    if p is Cat:
        return "cat"
    if p is Dog:
        return "dog"
    return "unknown"

let c = Cat("Whiskers")
let d = Dog("Rex", "Labrador")
print(pet_kind(c))   # cat
print(pet_kind(d))   # dog

# --- `is not None` narrowing on Optional ---
fn maybe_int() -> Option[int]:
    return 42

let v = maybe_int()
if v is not None:
    print("v has a value")   # v has a value

# --- `is` narrowing on Union type ---
fn get_union() -> Union[int, str]:
    return 10

let u = get_union()
if u is int:
    print("u is int")    # u is int
if u is str:
    print("u is str")    # (not printed)

# --- `is not` on Union narrows the type ---
fn get_optional() -> Option[str]:
    return "hello"

let s = get_optional()
if s is not None:
    print("s is not None")   # s is not None

# --- elif branches are narrowed the same way ---
fn get_opt_int() -> Option[int]:
    return 7

let w = get_opt_int()
if w is None:
    print("w is None")
elif w is not None:
    print("w has a value")   # w has a value (narrowed to int in this branch)

fn classify(x: Union[int, str, float]) -> str:
    if x is int:
        return "int"
    elif x is str:
        return "str"
    elif x is float:
        return "float"
    return "other"

print(classify(1))      # int
print(classify("hi"))   # str
print(classify(2.5))    # float

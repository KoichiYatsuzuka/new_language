# other_typing.tl — Union, Option, Any, type guards (is / is not), enum, and cast (=>)

# ============================================================
# 1. Any: accepts any value, no static narrowing
# ============================================================
print("=== Any ===")

fn identity(x: Any) -> Any:
    return x

print(identity(42))       # 42
print(identity("hello"))  # hello
print(identity(True))     # True

# ============================================================
# 2. Union: variable holds one of several concrete types
# ============================================================
print("=== Union ===")

fn describe_union(x: Union[int, str, bool]) -> str:
    if x is int:
        return "int"
    if x is str:
        return "str"
    if x is bool:
        return "bool"
    return "other"

print(describe_union(10))      # int
print(describe_union("hi"))    # str
print(describe_union(True))    # bool

# ============================================================
# 3. Option[T]: shorthand for Union[T, None]
# ============================================================
print("=== Option ===")

fn maybe_value(flag: bool) -> Option[int]:
    if flag:
        return 99
    return None

let v1 = maybe_value(True)
let v2 = maybe_value(False)

if v1 is not None:
    print("v1 has a value")   # v1 has a value
if v2 is None:
    print("v2 is None")       # v2 is None

# ============================================================
# 4. is: runtime type check against primitives and classes
# ============================================================
print("=== is ===")

fn type_of(x: Any) -> str:
    if x is int:
        return "int"
    if x is float:
        return "float"
    if x is str:
        return "str"
    if x is bool:
        return "bool"
    return "unknown"

print(type_of(1))      # int
print(type_of(1.5))    # float
print(type_of("a"))    # str
print(type_of(False))  # bool

print("=== is with classes ===")

class Cat:
    let name: str

class Dog:
    let name: str
    let breed: str

fn pet_sound(p: Any) -> str:
    if p is Cat:
        return "Meow"
    if p is Dog:
        return "Woof"
    return "..."

let cat = Cat("Whiskers")
let dog = Dog("Rex", "Labrador")
print(pet_sound(cat))  # Meow
print(pet_sound(dog))  # Woof

print("=== is with traits ===")

trait Flyable:
    fn fly(self) -> str:
        ...

class Bird(Flyable):
    let name: str
    fn fly(self) -> str:
        return "flap flap"

class Fish:
    let name: str

fn can_fly(x: Any) -> bool:
    return x is Flyable

let bird = Bird("Sparrow")
let fish = Fish("Nemo")
print(can_fly(bird))  # True
print(can_fly(fish))  # False

print("=== is function ===")

fn square(let n: int) -> int:
    return n * n

print(square is function)    # True
print(42 is function)        # False

# ============================================================
# 5. is not: narrowing on Union / Option
# ============================================================
print("=== is not narrowing ===")

fn print_if_present(x: Option[str]) -> None:
    if x is not None:
        print("value:", x)   # x narrowed to str here
    else:
        print("no value")

print_if_present("hello")  # value: hello
print_if_present(None)     # no value

print("=== elif narrowing on Union ===")

fn classify(x: Union[int, str, float]) -> str:
    if x is int:
        return "integer"
    elif x is str:
        return "string"
    elif x is float:
        return "float"
    return "other"

print(classify(7))      # integer
print(classify("yo"))   # string
print(classify(2.5))    # float

# ============================================================
# 6. enum
# ============================================================
print("=== enum: basic ===")

enum Color:
    Red
    Green
    Blue

let c = Color.Green
print(c.value)  # 1

# Explicit values; auto-numbering continues after the explicit value
enum MyEnum:
    a
    b = 5
    c

print(MyEnum.a.value)  # 0
print(MyEnum.b.value)  # 5
print(MyEnum.c.value)  # 6

print("=== enum: is check and equality ===")

let x = Color.Red
print(x is enum_item_Color)  # True

let r1 = Color.Red
let r2 = Color.Red
print(r1 == r2)          # True
print(r1 == Color.Blue)  # False

print("=== enum: match ===")

mut result = ""
match (c):
    case Color.Red:
        result = "red"
    case Color.Green:
        result = "green"
    case Color.Blue:
        result = "blue"
print(result)  # green

print("=== enum: iteration ===")

for item in [Color.Red, Color.Green, Color.Blue]:
    print(item.value)  # 0, 1, 2

# ============================================================
# 7. Cast operator (=>)  and  __cast__ special method
# ============================================================
print("=== cast: basic ===")

class Celsius:
    let value: float

    fn __init__(mut self, value: float)->None:
        self.value = value

    fn __cast__[float](let self)->float:
        return self.value

    fn __cast__[int](let self)->int:
        return int(self.value)

    fn __repr__(let self)->str:
        return "Celsius(" + repr(self.value) + ")"

let temp = Celsius(36.6)
let cast_f: float = temp=>float
let cast_i: int   = temp=>int
print(cast_f)   # 36.6
print(cast_i)   # 36

print("=== cast: chaining ===")

class JsonLike:
    let data: dict[str, int]

    fn __init__(mut self, data: dict[str, int])->None:
        self.data = data

    fn __cast__[dict[str, int]](let self)->dict[str, int]:
        return self.data

let jl = JsonLike(dict[str, int]({"a": 1, "b": 2}))
let jl_keys = jl=>dict[str, int].keys()
print(jl_keys)  # ["a", "b"]

print("=== cast: new_type ===")

new_type Meters: float
new_type Celsius2: float
new_type Count: int
new_type Kg: float

let m = 5.0=>Meters
let c2_val = 100.0=>Celsius2
print(repr(m))      # Meters(5.0)
print(repr(c2_val)) # Celsius2(100.0)

let cnt = 42=>Count
print(repr(cnt))    # Count(42)

let raw_f = m=>float
let raw_i = cnt=>int
print(raw_f)        # 5.0
print(raw_i)        # 42

let kg = m=>Kg
print(repr(kg))     # Kg(5.0)

print("=== cast: auto-cast for let parameters ===")

class Wrapper:
    let val: int

    fn __init__(mut self, val: int)->None:
        self.val = val

    fn __cast__[int](let self)->int:
        return self.val

fn double(let x: int)->int:
    return x * 2

let w = Wrapper(21)
print(double(w))    # 42

print("=== cast: multiple targets ===")

class CastPoint:
    let x: int
    let y: int

    fn __init__(mut self, x: int, y: int)->None:
        self.x = x
        self.y = y

    fn __cast__[str](let self)->str:
        return "(" + repr(self.x) + ", " + repr(self.y) + ")"

    fn __cast__[list](let self)->list:
        return [self.x, self.y]

let cp = CastPoint(3, 4)
print(cp=>str)   # (3, 4)
print(cp=>list)  # [3, 4]

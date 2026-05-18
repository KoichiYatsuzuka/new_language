# other_typing.tl — Union, Option, Any, type guards (is / is not), and enum

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

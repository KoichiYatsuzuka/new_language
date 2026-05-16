# subtype.tl — Union, Option, Any, and type guard narrowing (is / is not)

# --- Any: accepts any value, no static narrowing ---
print("=== Any ===")

fn identity(x: Any) -> Any:
    return x

print(identity(42))       # 42
print(identity("hello"))  # hello
print(identity(True))     # True

# --- Union: variable holds one of several concrete types ---
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

# --- Option[T]: shorthand for Union[T, None] ---
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

# --- is: runtime type check, returns bool ---
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

# --- is with class types ---
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

# --- is with trait membership ---
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

# --- is not: narrowing on Union/Option ---
print("=== is not narrowing ===")

fn print_if_present(x: Option[str]) -> None:
    if x is not None:
        print("value:", x)   # x narrowed to str here
    else:
        print("no value")

print_if_present("hello")  # value: hello
print_if_present(None)     # no value

# --- elif chain narrowing on Union ---
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

# --- is function: check if a value is callable ---
print("=== is function ===")

fn square(let n: int) -> int:
    return n * n

print(square is function)    # True
print(42 is function)        # False

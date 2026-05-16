# polymorphism.tl — templates, new_type, and trait-based polymorphism

# --- Trait definitions used for constraints ---
trait Printable:
    fn to_str(self) -> str:
        ...

trait Comparable:
    fn less_than(self, other: Self) -> bool:
        ...

# --- Classes implementing traits ---
class MyInt(Printable, Comparable):
    mut value: int

    fn to_str(self) -> str:
        return "MyInt"

    fn less_than(self, other: Self) -> bool:
        return self.value < other.value

class MyStr(Printable):
    mut text: str

    fn to_str(self) -> str:
        return "MyStr(" + self.text + ")"

# --- Template function: single trait constraint ---
print("=== template function ===")

fn show[T: Printable](item: T) -> None:
    print(item.to_str())

let a = MyInt(42)
let b = MyStr("hello")
show[MyInt](a)   # MyInt
show[MyStr](b)   # MyStr(hello)

# --- Template function: multiple constraints (and) ---
print("=== template with multiple constraints ===")

fn min_of[T: Printable and Comparable](x: T, y: T) -> T:
    if x.less_than(y):
        return x
    return y

let x = MyInt(10)
let y = MyInt(30)
let smaller = min_of[MyInt](x, y)
print(smaller.to_str())  # MyInt(10)

# --- Template class ---
print("=== template class ===")

class Box[T: Printable]:
    mut item: T

    fn label(self) -> str:
        return "Box[" + self.item.to_str() + "]"

let box_int = Box[MyInt](a)
let box_str = Box[MyStr](b)
print(box_int.label())   # Box[MyInt(42)]
print(box_str.label())   # Box[MyStr(hello)]

# --- new_type from class: creates a distinct type sharing all methods ---
print("=== new_type from class ===")

class Meters:
    mut value: int

    fn get(self) -> int:
        return self.value

    fn add(self, other: Self) -> Self:
        return Self(self.value + other.value)

    fn double(self) -> Self:
        return Self(self.value * 2)

new_type Kilometers: Meters
new_type LightSeconds: Meters

let m  = Meters(100)
let km = Kilometers(5)
let ls = LightSeconds(3)

print(m.get())   # 100
print(km.get())  # 5
print(ls.get())  # 3

# Methods return the correct new_type, not Meters
let km2 = km.double()
print(km2.get())  # 10

# new_types of the same base cannot be mixed via add (Self-typed)
# km.add(m)   # TypeError: expected Kilometers, got Meters

# --- new_type from primitive ---
print("=== new_type from primitive ===")

new_type Celsius: int
new_type Fahrenheit: int

let c = Celsius(100)
let f = Fahrenheit(212)
print(c.value)   # 100
print(f.value)   # 212

# Celsius and Fahrenheit are distinct — cannot be mixed
# let bad = c.value + f.value  # allowed here since we extract int first
let c_int = c.value
let f_int = f.value
print(c_int + f_int)  # 312 (plain ints can be added)

# --- Trait-based polymorphism: dispatch via a common interface ---
print("=== trait-based polymorphism ===")

fn print_all[T: Printable](items: T) -> None:
    print(items.to_str())

print_all[MyInt](MyInt(1))
print_all[MyInt](MyInt(2))
print_all[MyStr](MyStr("a"))

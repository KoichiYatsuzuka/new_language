# enum behavior examples

# Basic enum with auto-numbering (0, 1, 2, ...)
enum Color:
    Red
    Green
    Blue

let c = Color.Green
print(c.value)  # 1

# Enum with explicit values; auto-numbering continues after explicit value
enum MyEnum:
    a
    b = 5
    c

print(MyEnum.a.value)  # 0
print(MyEnum.b.value)  # 5
print(MyEnum.c.value)  # 6

# Enum variants are instances of enum_item_<Name>
let x = Color.Red
print(x is enum_item_Color)  # True

# Each class-var access returns the same Rc, so == works
let r1 = Color.Red
let r2 = Color.Red
print(r1 == r2)   # True
print(r1 == Color.Blue)  # False

# match against enum values
mut result = ""
match (c):
    case Color.Red:
        result = "red"
    case Color.Green:
        result = "green"
    case Color.Blue:
        result = "blue"
print(result)  # green

# Enum members accessed via the class object
for item in [Color.Red, Color.Green, Color.Blue]:
    print(item.value)

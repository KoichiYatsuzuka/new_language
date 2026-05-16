# match statement examples

# --- case: value comparison ---

let x = 2
match (x):
    case 1:
        print("one")
    case 2:
        print("two")
    case 3:
        print("three")
    case _:
        print("other")

# case with strings
let s = "hello"
match (s):
    case "world":
        print("world")
    case "hello":
        print("hello")
    case _:
        print("unknown")

# --- is: type check ---

let v = 42
match (v):
    is int:
        print("int")
    is str:
        print("str")
    is float:
        print("float")

# --- block: expression with match to compute a value ---

let score = 85
let grade = block:
    match (score // 10):
        case 10:
            block_return "A+"
        case 9:
            block_return "A"
        case 8:
            block_return "B"
        case 7:
            block_return "C"
        case _:
            block_return "F"
    block_return "F"
print(grade)

# --- block_return inside match exits the enclosing block: early ---

mut result = 0
block:
    match (1):
        case 1:
            result = 42
            block_return 0
    result = 999
print(result)

# --- is with classes ---

class Dog:
    fn bark(self) -> str:
        return "woof"

class Cat:
    fn meow(self) -> str:
        return "meow"

mut animal = Dog()
match (animal):
    is Dog:
        print("it's a dog")
    is Cat:
        print("it's a cat")

# --- wildcard only ---

let n = 999
match (n):
    case _:
        print("caught by wildcard")

# --- no arm matches (no error, just does nothing) ---

let y = 7
match (y):
    case 1:
        print("one")
    case 2:
        print("two")

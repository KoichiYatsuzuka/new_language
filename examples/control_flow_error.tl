# control_flow_error.tl — expected control-flow errors
#
# Section 1 — StaticTypeError: comparison between incompatible types
#   Comparing str with int using < is a type error.
#   Comparing bool with int using > is a type error.

let name = "Alice"
let age  = 30
if name < age:   # StaticTypeError: str < int
    print("unreachable")

let flag = True
if flag > 0:     # StaticTypeError: bool > int
    print("unreachable")

# Section 2 — ParseError: match statement cannot mix 'case' and 'is' arms

mut x = 0
match (x):
    case 0:
        print("zero")
    is int:
        print("int")

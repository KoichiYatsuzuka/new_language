# control_flow_error.tl — StaticTypeError: comparison between incompatible types
#
# Expected errors (StaticTypeError, all reported together before execution):
#   Comparing str with int using < is a type error.
#   Comparing bool with int using > is a type error.

let name = "Alice"
let age  = 30
if name < age:   # StaticTypeError: str < int
    print("unreachable")

let flag = True
if flag > 0:     # StaticTypeError: bool > int
    print("unreachable")

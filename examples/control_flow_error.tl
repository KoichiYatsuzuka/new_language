# control_flow_error.tl — expected control-flow errors
#
# Section 1 — StaticTypeError: comparison between incompatible types
# Section 2 — ParseError: match cannot mix 'case' and 'is' arms
# Section 3 — NameError: block-scoped variable accessed outside its block
# Section 4 — TypeError: block_return value mismatches ->Type annotation

let name = "Alice"
let age  = 30
if name < age:   # StaticTypeError: str < int
    print("unreachable")

let flag = True
if flag > 0:     # StaticTypeError: bool > int
    print("unreachable")

# Section 2 — ParseError: match statement cannot mix 'case' and 'is' arms
# (comment out Section 1 to reach this)

# mut x = 0
# match (x):
#     case 0:
#         print("zero")
#     is int:
#         print("int")

# Section 3 — NameError: block-scoped variable accessed outside its block
# (comment out earlier sections to reach this)

# block:
#     mut inner = 42
#     print("inside block:", inner)   # 42
#
# print(inner)   # NameError: 'inner' is not defined

# Section 4 — TypeError: block_return value mismatches ->Type annotation
# (comment out earlier sections to reach this)

# let bad = block ->int:
#     block_return "not an int"   # TypeError: 'str' returned where 'int' expected
#
# # block_return int inside if ->str:
# # let bad2 = if True ->str:
# #     block_return 999   # TypeError: 'int' returned where 'str' expected
#
# # block_return None inside block ->int:
# # let bad3 = block ->int:
# #     block_return None   # TypeError: 'NoneType' returned where 'int' expected
#
# # block_return wrong type in match ->int:
# # let bad4 = match (1) ->int:
# #     case 1:
# #         block_return "one"   # TypeError: 'str' returned where 'int' expected

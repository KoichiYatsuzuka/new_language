# block_return_type_check__errors.tl — TypeError for mismatched block_return annotations
#
# Each section raises a TypeError at the point of block_return.
# Comment out earlier sections to reach later ones.

# --- Error 1: block_return str inside block ->int ---
# Expected: TypeError: block_return value has type 'str', but 'int' was expected

let x = block ->int:
    block_return "not an int"   # TypeError
print(x)

# --- Error 2: block_return int inside if ->str (comment out Error 1 to reach) ---

# let y = if True ->str:
#     block_return 999   # TypeError: 'int' returned where 'str' expected
# print(y)

# --- Error 3: block_return None inside block ->int (comment out Error 1 to reach) ---

# let z = block ->int:
#     block_return None   # TypeError: 'NoneType' returned where 'int' expected
# print(z)

# --- Error 4: block_return wrong type in match ->int (comment out Error 1 to reach) ---

# let w = match (1) ->int:
#     case 1:
#         block_return "one"   # TypeError: 'str' returned where 'int' expected
# print(w)

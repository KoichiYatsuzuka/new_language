# functions__errors.tl — expected function-related errors
#
# Section 1 — ParseError: non-default parameter after a default parameter
#   Expected: ParseError at parse time
#
# Section 2 — TypeError: freeze on a mut variable captured by a closure
#   Expected: runtime TypeError
#
# Note: ParseError is detected before execution, so only one section
# can be demonstrated per run. Comment out the other to test each.

# --- Section 1: ParseError ---
# A required parameter may not follow an optional one.

fn my_func(let param1: int = 0, let param2: int) -> int:
    return param1 + param2

# --- Section 2: TypeError (comment out Section 1 to reach this) ---
# Freezing a mut variable that an inner function has captured is forbidden
# because the inner function still holds a reference to the shared cell.

# fn make_counter() -> None:
#     mut count = 0
#     fn inc() -> None:
#         count += 1
#     freeze count   # TypeError: cannot freeze 'count' because it is captured by a closure
#
# make_counter()

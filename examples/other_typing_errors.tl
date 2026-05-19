# other_typing__errors.tl — expected typing errors
#
# Section 1 — StaticTypeError: `is not` on a non-Union type
#   Expected: StaticTypeError reported before execution
#
# Section 2 — ParseError/TypeError: enum variant value must be an integer
#   Expected: error at parse or type-check time

# --- Section 1: StaticTypeError ---
# `x is not T` requires x to be a Union or Option type.
# Using it on a plain (non-Union) type is a static error.

fn check(x: int) -> None:
    if x is not None:   # StaticTypeError: IsNotOnNonUnion — x is int, not Union/Option
        print(x)

check(42)

# --- Section 2: enum variant value must be an integer (comment out Section 1 to reach this) ---

# enum BadEnum:
#     a = "not_an_int"

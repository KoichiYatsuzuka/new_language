# subtype_error.tl — StaticTypeError: is not on a non-Union type
#
# Expected error (StaticTypeError, reported before execution):
#   `x is not T` requires x to be a Union or Option type.
#   Using it on a plain (non-Union) type is a static error.

fn check(x: int) -> None:
    if x is not None:   # StaticTypeError: IsNotOnNonUnion — x is int, not Union/Option
        print(x)

check(42)

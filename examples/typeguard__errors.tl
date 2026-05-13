# typeguard__errors.tl — Demonstrates static errors for invalid `is not` usage
# Expected error: StaticTypeError: 'is not' type guard on 'x' requires a Union or Optional type

fn check(x: int) -> str:
    if x is not str:
        return "not a string"
    return "string"

let r = check(1)
print(r)

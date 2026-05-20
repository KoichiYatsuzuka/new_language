# cast__errors.tl — verifies that cast errors are raised correctly
#
# Each block should raise an error; run them one at a time.

# ── Error: no __cast__ method for the target type ────────────────────────────

class Foo:
    let x: int
    fn __init__(mut self, x: int)->None:
        self.x = x
    fn __cast__[int](let self)->int:
        return self.x

let f = Foo(5)
# Foo has __cast__[int] but NOT __cast__[str]
let s = f=>str   # TypeError: 'Foo' is not castable to 'str'
print(s)

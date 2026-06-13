# mod_uses_tl.tl — module that imports another tl module
# Tests: can --compile handle import[tl]?

import tl_math as tm

# Calls tl_math.add — NOT eligible for native (cross-module Attr call)
fn double_add(a: int, b: int) -> int:
    return tm.add(a, b) + tm.add(a, b)

# No cross-module calls — eligible for native compilation
fn square(n: int) -> int:
    return n * n

fn cube(n: int) -> int:
    return n * n * n

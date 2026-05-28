# mod_uses_py.tl — module that imports a py (converter) module
# Tests: can --compile handle import[py]?

import[py] helper as h

# Calls helper.multiply — NOT eligible for native (cross-module call)
fn scaled(n: int, factor: int) -> int:
    return h.multiply(n, factor)

# No py calls — eligible for native compilation
fn abs_val(n: int) -> int:
    if n < 0:
        return -n
    return n

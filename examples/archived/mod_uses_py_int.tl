# mod_uses_py_int.tl — module that imports a py-int module
# Tests: can --compile handle import[py-int]?

import[py-int] time

# Calls time.time() — NOT eligible for native (cross-module call)
fn timed_square(n: int) -> int:
    let t0 = time.time()
    let result = n * n
    let t1 = time.time()
    return result

# No py-int calls — eligible for native compilation
fn factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)

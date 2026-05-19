# native_ops.tl — a module designed for partial native compilation.
#
# Every function here carries explicit int/float type annotations.
# The compiler uses those annotations to generate direct Rust arithmetic
# instead of routing each operation through the callback ABI, giving
# 100–200× speedup on tight loops compared to the tree-walk interpreter.
#
# Compile:
#   cargo run --release -- --compile examples/native_ops.tl
#
# Eligible functions are those with:
#   - explicit int / float parameters and return type
#   - no closures, generators, try/raise, or block_return/loop_yield

# ── Integer functions ─────────────────────────────────────────────────────────

fn fib(n: int) -> int:
    mut a = 0
    mut b = 1
    mut i = 0
    while i < n:
        let t = b
        b = a + b
        a = t
        i += 1
    return a

fn count_divisors(n: int) -> int:
    mut count = 0
    mut i = 1
    while i * i <= n:
        if n % i == 0:
            count += 2
            if i * i == n:
                count -= 1
        i += 1
    return count

fn digit_sum(mut n: int) -> int:
    mut total = 0
    while n > 0:
        total += n % 10
        n = n // 10
    return total

# ── Float functions ───────────────────────────────────────────────────────────

fn approx_sqrt(x: float) -> float:
    if x <= 0.0:
        return 0.0
    mut g = x * 0.5
    mut i = 0
    while i < 30:
        g = (g + x / g) * 0.5
        i += 1
    return g

fn sum_inv_squares_term(i: int) -> float:
    let fi = i * 1.0
    return 1.0 / (fi * fi)

fn sum_inv_squares(n: int) -> float:
    mut total = 0.0
    mut i = 1
    while i <= n:
        total = total + sum_inv_squares_term(i)
        i += 1
    return total

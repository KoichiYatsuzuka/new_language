# bench_heavy.tl — confirm speedup for int, float, str, and class after compilation.
#
# Workflow:
#   Step 1  (tree-walk baseline):
#     cargo run --release -- examples/bench_heavy.tl
#
#   Step 2  (compile the module):
#     cargo run --release -- --compile examples/heavy_ops.tl
#
#   Step 3  (native dispatch — same bench file):
#     cargo run --release -- examples/bench_heavy.tl

import[py-int] time
import heavy_ops

fn elapsed(t0: float, t1: float) -> str:
    return "elapsed: " + str(t1 - t0) + " s"

# ── int: prime counting ────────────────────────────────────────────────────────
# is_prime / count_primes use intra-module calls; every arithmetic op stays native.
print("=== int: prime counting (limit=80000) ===")
let t0 = time.time()
let n_primes = heavy_ops.count_primes(80000)
let t1 = time.time()
print("count_primes(80000):", n_primes)
print(elapsed(t0, t1))
print("")

# ── int: Collatz total steps ───────────────────────────────────────────────────
# collatz_steps / total_collatz: tight integer loops with branching.
print("=== int: Collatz total steps (limit=50000) ===")
let t2 = time.time()
let n_collatz = heavy_ops.total_collatz(50000)
let t3 = time.time()
print("total_collatz(50000):", n_collatz)
print(elapsed(t2, t3))
print("")

# ── float: Newton sqrt sum ─────────────────────────────────────────────────────
# Each of the 100000 calls runs 20 Newton iterations — heavy float arithmetic.
print("=== float: Newton sqrt sum (n=100000) ===")
let t4 = time.time()
let sqrt_sum = heavy_ops.sum_sqrts(100000)
let t5 = time.time()
print("sum_sqrts(100000):", sqrt_sum)
print(elapsed(t4, t5))
print("")

# ── float: Leibniz pi series ───────────────────────────────────────────────────
# 2000000 iterations of alternating-sign division: pure float loop.
print("=== float: Leibniz pi series (2000000 terms) ===")
let t6 = time.time()
let pi_approx = heavy_ops.series_pi(2000000)
let t7 = time.time()
print("series_pi(2000000) ≈", pi_approx)
print(elapsed(t6, t7))
print("")

# ── str: vowel counting ────────────────────────────────────────────────────────
# 20000 strings × 43 chars each; is_vowel is an intra-module call per char.
print("=== str: vowel counting (20000 strings) ===")
let words = for i in range(20000) ->list[str]:
    loop_yield "the quick brown fox jumps over the lazy dog"
let t8 = time.time()
let total_v = heavy_ops.total_vowels(words)
let t9 = time.time()
print("total_vowels:", total_v)
print(elapsed(t8, t9))
print("")

# ── class: Mandelbrot ──────────────────────────────────────────────────────────
# 200×200 grid of Complex points; mandelbrot_iter is an intra-module call that
# does up to 100 float operations per point — class used as a thin container.
print("=== class: Mandelbrot (200x200 grid, max_iter=100) ===")
let N = 200
let points = for idx in range(N * N) ->list[Complex]:
    let px = idx % N
    let py = idx // N
    loop_yield heavy_ops.Complex(px * 0.015 - 2.5, py * 0.015 - 1.5)
let t10 = time.time()
let mb_sum = heavy_ops.mandelbrot_sum(points, 100, N * N)
let t11 = time.time()
print("mandelbrot_sum:", mb_sum)
print(elapsed(t10, t11))

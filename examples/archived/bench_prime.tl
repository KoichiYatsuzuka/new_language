# bench_prime.tl — benchmark: prime factorisation timing
#
# The module is loaded as interpreted (.tl) or compiled (.tlc) depending on
# what exists on disk.  Run the two scenarios in order:
#
#   Step 1 — interpreted:
#     cargo run --release -- examples/bench_prime.tl
#
#   Step 2 — compile the module:
#     cargo run --release -- --compile examples/prime_factors.tl
#
#   Step 3 — compiled module (same bench file, module is now .tlc):
#     cargo run --release -- examples/bench_prime.tl
#
# Expected: times are similar for now because .tlc embeds the source text and
# the interpreter still executes it.  They will diverge once LLVM IR
# code-generation is implemented and the .tlc contains native machine code.
#
# Target number: 999962000357 = 999979 × 999983  (two six-digit primes)
# Trial-division checks ~499 990 odd divisors, so it takes noticeable time.

import[py-int] time
import prime_factors

let n = 999962000357

print("Target :", n)
print("Searching for prime factors (trial division up to sqrt(n) ~999981)...")

let t0 = time.time()

mut factors = 0
mut product = 1
for f in prime_factors.factorize(n):
    factors += 1
    product = product * f
    print("  factor:", f)

let elapsed = time.time() - t0

print("Factors found    :", factors)
print("Product check    :", product, "==", n, "->", product == n)
print("Elapsed          :", elapsed, "seconds")

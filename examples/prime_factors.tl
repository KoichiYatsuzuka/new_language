# prime_factors.tl — module: prime factorisation by trial division
#
# Usage (as a module):
#   import prime_factors
#   for f in prime_factors.factorize(n):
#       print(f)
#
# Compile:
#   cargo run -- --compile examples/prime_factors.tl

gen factorize(n: int) -> int:
    mut m = n
    while m % 2 == 0:
        yield 2
        m = m // 2
    mut d = 3
    while d * d <= m:
        while m % d == 0:
            yield d
            m = m // d
        d += 2
    if m > 1:
        yield m

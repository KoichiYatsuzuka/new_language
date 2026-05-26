# libm_demo.tl — Import the libm crate (pure-Rust math) via import[rs].
#
# The interpreter reads libm's source from the cargo registry, auto-discovers
# all compatible pub fn, and compiles call-through wrappers.  No .rs file needed.
#
#   cargo run --release -- examples/libm_demo.tl

import[rs] libm[0.2]

let pi: float = 3.141592653589793

print("=== libm demo ===")
print(f"sin(pi/2)      = {libm.sin(pi / 2.0)}")
print(f"cos(0)         = {libm.cos(0.0)}")
print(f"sqrt(2)        = {libm.sqrt(2.0)}")
print(f"pow(2, 10)     = {libm.pow(2.0, 10.0)}")
print(f"log(e)         = {libm.log(2.718281828459045)}")
print(f"exp(1)         = {libm.exp(1.0)}")
print(f"fabs(-3.14)    = {libm.fabs(-3.14)}")
print(f"floor(2.9)     = {libm.floor(2.9)}")
print(f"ceil(2.1)      = {libm.ceil(2.1)}")

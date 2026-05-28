# importation.tl — demonstrates all import styles and qualifiers.
#
# Import styles:
#   import             — auto: prefers .tlc if present, falls back to .tl
#   import[tl]         — force .tl source (tree-walk, ignores .tlc)
#   import[tlc]        — force .tlc compiled (error if no .tlc exists)
#   import[py-int]     — Python module with integer conversion
#   import[rs]         — Rust crate (auto-discovered or declared in tl_crates.json)
#   from … import      — selective import from a .tl module
#   subdir.module      — subdirectory .tl module
#
# Prerequisites for native sections:
#   cargo run --release -- --compile examples/test_modules/native_ops.tl

import[py-int] time

# ── 1. Import qualifiers: auto / [tl] / [tlc] ────────────────────────────────

import        test_modules.native_ops as auto_mod
import[tl]    test_modules.native_ops as tl_mod
import[tlc]   test_modules.native_ops as tlc_mod

let t0 = time.time()
let r_auto = auto_mod.fib(40)
let t1 = time.time()
print("auto  fib(40):", r_auto, "  elapsed:", t1 - t0, "s")

let t2 = time.time()
let r_tl = tl_mod.fib(40)
let t3 = time.time()
print("[tl]  fib(40):", r_tl, "  elapsed:", t3 - t2, "s")

let t4 = time.time()
let r_tlc = tlc_mod.fib(40)
let t5 = time.time()
print("[tlc] fib(40):", r_tlc, "  elapsed:", t5 - t4, "s")

print("all results equal:", r_auto == r_tl, r_tl == r_tlc)

# ── 2. Native module — correctness and timing ─────────────────────────────────

print("=== correctness ===")
print("fib(30)              :", auto_mod.fib(30))           # 832040
print("fib(50)              :", auto_mod.fib(50))           # 12586269025
print("count_divisors(720)  :", auto_mod.count_divisors(720))   # 30
print("digit_sum(123456789) :", auto_mod.digit_sum(123456789))  # 45
print("approx_sqrt(2.0)     :", auto_mod.approx_sqrt(2.0))      # ~1.41421356
print("sum_inv_squares(10)  :", auto_mod.sum_inv_squares(10))   # ~1.54977

print("=== timing ===")
let t6 = time.time()
mut k = 0
while k < 1000:
    let _ = auto_mod.fib(60)
    k += 1
let t7 = time.time()
print("1000 × fib(60)  elapsed:", t7 - t6, "s")

let t8 = time.time()
let pi_sq_over_6 = auto_mod.sum_inv_squares(500000)
let t9 = time.time()
print("sum_inv_squares(500000):", pi_sq_over_6, "  elapsed:", t9 - t8, "s")

# ── 3. import[py-int] — pandas ───────────────────────────────────────────────

import[py-int] warnings as w
w.simplefilter("ignore")   # suppress pandas CoW FutureWarning

import[py-int] pandas as pd

let df = pd.DataFrame({
    "name":  ["Alice", "Bob", "Charlie", "Diana", "Eve"],
    "age":   [25, 30, 35, 28, 22],
    "score": [85, 92, 78, 95, 88]
})

print("=== DataFrame ===")
print(df)
print("shape:", df.shape)
print("rows :", len(df))

let scores = df["score"]
print("score mean:", scores.mean())
print("score max :", scores.max())
print("describe  :")
print(df.describe())

let mask = scores > 88
let high = df[mask]
print("score > 88 rows:")
print(high)

df["pass"] = df["score"] >= 85
print("with pass column:")
print(df)

print("score values:")
for v in scores:
    print(v)

let sorted_df = df.sort_values("score", ascending=False)
print("top 3 by score:")
print(sorted_df.head(3))

# ── 4. import[py-int] — PyObject operators and iteration ─────────────────────

import[py-int] test_modules.py_calculator as calc

# binary operators
let c = calc.make_container([1, 2])
let c2 = c * 3               # Container.__mul__
print(len(c2))               # 6

let c3 = 3 * c               # Container.__rmul__
print(len(c3))               # 6

let a = calc.make_container([1, 2])
let b = calc.make_container([3, 4, 5])
let ab = a + b               # Container.__add__
print(len(ab))               # 5

mut total = 0
for v in ab:
    total += v
print(total)                 # 15

# iteration
let c_iter = calc.make_container([10, 20, 30, 40])
mut sum_iter = 0
for x in c_iter:
    sum_iter += x
print(sum_iter)              # 100

let outer = calc.make_container([1, 2, 3])
mut result = 0
for v2 in outer:
    result += v2 * 10
print(result)                # 60

# ── 5. import / from … import — .tl module ───────────────────────────────────

import tl_math as tm

let tl_result: int = tm.add(3, 4)
print(tl_result)

let msg: str = tm.greet("World")
print(msg)

print(tm.PI)

from tl_math import add, greet

let r2: int = add(10, 20)
print(r2)

let m2: str = greet("TL")
print(m2)

import geometry as geo

let area_a: float = geo.area_circle(5.0)
print(area_a)

let area_b: float = geo.area_rect(3.0, 4.0)
print(area_b)

print(geo.DIMS)

# subdirectory module — dot-separated path components map to filesystem subdirs
import subdir.sub_module as sub

let sub_result: int = sub.compute(10, 32)
print(sub_result)
print(sub.LABEL)

# ── 6. import[rs] — Rust crates ──────────────────────────────────────────────

# libm — pure-Rust math functions (auto-discovered from cargo registry)
import[rs] libm[0.2]

let pi: float = 3.141592653589793

print("=== libm ===")
print(f"sin(pi/2)   = {libm.sin(pi / 2.0)}")
print(f"cos(0)      = {libm.cos(0.0)}")
print(f"sqrt(2)     = {libm.sqrt(2.0)}")
print(f"pow(2, 10)  = {libm.pow(2.0, 10.0)}")
print(f"log(e)      = {libm.log(2.718281828459045)}")
print(f"exp(1)      = {libm.exp(1.0)}")
print(f"fabs(-3.14) = {libm.fabs(-3.14)}")
print(f"floor(2.9)  = {libm.floor(2.9)}")
print(f"ceil(2.1)   = {libm.ceil(2.1)}")

# rand_ops — random number generation (declared in tl_crates.json)
import[rs] rand_ops

print("=== rand_ops ===")
mut ri = 0
while ri < 5:
    print(f"  float={rand_ops.random_float()}  int={rand_ops.random_int()}  bool={rand_ops.random_bool()}")
    ri += 1

# sha2_ops — SHA-256/512 hashing (declared in tl_crates.json)
import[rs] sha2_ops

let msg = "hello, world"
print("=== sha2_ops ===")
let h256: str = sha2_ops.sha256(msg)
print(f"SHA-256({msg}) = {h256}")
let h512: str = sha2_ops.sha512(msg)
print(f"SHA-512({msg}) = {h512}")

# bench_point.tl — benchmark: class instance operations (interpreted vs compiled)
#
# Measures the speed of functions that operate on a list of Point instances.
# The `point_ops` module is loaded as source (.tl) or compiled (.tlc)
# depending on what exists on disk — the same bench file covers both cases.
#
# Workflow
# --------
#   Step 1  (tree-walk / source module):
#     cargo run --release -- examples/bench_point.tl
#
#   Step 2  (compile the module):
#     cargo run --release -- --compile examples/point_ops.tl
#
#   Step 3  (compiled module — same bench file, parser now prefers .tlc):
#     cargo run --release -- examples/bench_point.tl
#
# What is compiled in Step 2:
#   sum_sq, count_inside, centroid_x, centroid_y, max_dist_sq  (all eligible)
#   Point class methods (dot, dist_sq) are NOT compiled — they live inside
#   a ClassDef and are always tree-walked.
#
# Expected: Steps 1 and 3 show the same numeric results; Step 3 is faster.
# Observed speedup (N=100 000, release build):
#   Tree-walk total : ~0.79 s
#   Native total    : ~0.28 s   (≈ 2.8× faster)
# Per-function ratios are 2.5–3.3×. The residual overhead comes from attribute
# accesses (p.x, p.y) routing through interpreter callbacks; the gain comes
# from native loop control, branching, and local-variable management.

import[py-int] time
import point_ops

let N = 100000

print("Building", N, "Point instances ...")
let t_build0 = time.time()

# Build a list of N points using a for-expression with loop_yield.
# x = i % 1000,  y = (i * 7 + 13) % 1000  — pseudo-random spread.
let points = for i in range(N) ->list[Point]:
    loop_yield point_ops.Point(i % 1000, (i * 7 + 13) % 1000)

let t_build1 = time.time()
print("Build time :", t_build1 - t_build0, "s")
print("")

# ── sum_sq ────────────────────────────────────────────────────────────────────
print("sum_sq (" , N , "points) ...")
let t0 = time.time()
let result_sq = point_ops.sum_sq(points)
let t1 = time.time()
print("  result  :", result_sq)
print("  elapsed :", t1 - t0, "s")
print("")

# ── count_inside ──────────────────────────────────────────────────────────────
let radius = 700
let r_sq   = radius * radius
print("count_inside (radius =", radius, ") ...")
let t2 = time.time()
let result_ci = point_ops.count_inside(points, r_sq)
let t3 = time.time()
print("  result  :", result_ci, "points inside")
print("  elapsed :", t3 - t2, "s")
print("")

# ── centroid ──────────────────────────────────────────────────────────────────
print("centroid ...")
let t4 = time.time()
let cx = point_ops.centroid_x(points, N)
let cy = point_ops.centroid_y(points, N)
let t5 = time.time()
print("  centroid: (", cx, ",", cy, ")")
print("  elapsed :", t5 - t4, "s")
print("")

# ── max_dist_sq ───────────────────────────────────────────────────────────────
print("max_dist_sq ...")
let t6 = time.time()
let result_md = point_ops.max_dist_sq(points)
let t7 = time.time()
print("  result  :", result_md)
print("  elapsed :", t7 - t6, "s")
print("")

print("Total (all four ops):", (t1 - t0) + (t3 - t2) + (t5 - t4) + (t7 - t6), "s")

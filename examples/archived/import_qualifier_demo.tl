# import_qualifier_demo.tl — demonstrates [tl] and [tlc] import qualifiers.
#
# import native_ops          — auto: prefers .tlc if present, falls back to .tl
# import[tl]  native_ops    — force .tl source (tree-walk, ignores .tlc)
# import[tlc] native_ops    — force .tlc compiled (error if no .tlc exists)
#
# Prerequisites:
#   cargo run --release -- --compile examples/native_ops.tl

import[py-int] time

# ── auto (default) ────────────────────────────────────────────────────────────
import native_ops as auto_mod

let t0 = time.time()
let r_auto = auto_mod.fib(40)
let t1 = time.time()
print("auto  fib(40):", r_auto, "  elapsed:", t1 - t0, "s")

# ── force .tl source ──────────────────────────────────────────────────────────
import[tl] native_ops as tl_mod

let t2 = time.time()
let r_tl = tl_mod.fib(40)
let t3 = time.time()
print("[tl]  fib(40):", r_tl, "  elapsed:", t3 - t2, "s")

# ── force .tlc compiled ───────────────────────────────────────────────────────
import[tlc] native_ops as tlc_mod

let t4 = time.time()
let r_tlc = tlc_mod.fib(40)
let t5 = time.time()
print("[tlc] fib(40):", r_tlc, "  elapsed:", t5 - t4, "s")

# ── all results must agree ────────────────────────────────────────────────────
print("")
print("all results equal:", r_auto == r_tl, r_tl == r_tlc)

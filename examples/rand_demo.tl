# rand_demo.tl — Import rand crate declared in tl_crates.json.
#
#   cargo run --release -- examples/rand_demo.tl

import[rs] rand_ops

mut i = 0
while i < 5:
    print(f"  float={rand_ops.random_float()}  int={rand_ops.random_int()}  bool={rand_ops.random_bool()}")
    i += 1

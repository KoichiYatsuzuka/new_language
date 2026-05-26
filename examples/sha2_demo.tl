# sha2_demo.tl — Import sha2 + hex crates declared in tl_crates.json.
#
# No .rs file needed — function bindings are defined entirely in tl_crates.json.
#
#   cargo run --release -- examples/sha2_demo.tl

import[rs] sha2_ops

let msg = "hello, world"

let h256: str = sha2_ops.sha256(msg)
print(f"SHA-256({msg}) =")
print(f"  {h256}")

let h512: str = sha2_ops.sha512(msg)
print(f"SHA-512({msg}) =")
print(f"  {h512}")

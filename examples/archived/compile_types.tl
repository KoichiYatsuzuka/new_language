# compile_types.tl — demonstrates native compilation with ALL value types.
#
# Every top-level function here is eligible for native compilation via the
# handle-based ABI (interpreter/native_api.rs).  Class instances, floats,
# strings, lists, and dicts all pass through the ABI as opaque i64 handles;
# the interpreter owns and operates on the actual values.
#
# Compile:
#   cargo run --release -- --compile examples/compile_types.tl

# ── Classes used by compiled functions ───────────────────────────────────────

class Vec3:
    mut x: float
    mut y: float
    mut z: float

class Pair:
    mut key: str
    mut val: int

# ── int / bool ────────────────────────────────────────────────────────────────

fn gcd(mut a: int, mut b: int) -> int:
    while b != 0:
        let t = b
        b = a % b
        a = t
    return a

fn is_even(n: int) -> bool:
    return n % 2 == 0

# ── float ─────────────────────────────────────────────────────────────────────

fn dot3(ax: float, ay: float, az: float,
        bx: float, by: float, bz: float) -> float:
    return ax * bx + ay * by + az * bz

fn vec3_len_sq(v: Vec3) -> float:
    let x = v.x
    let y = v.y
    let z = v.z
    return x * x + y * y + z * z

# ── str ───────────────────────────────────────────────────────────────────────

fn count_chars(s: str, n: int) -> int:
    mut count = 0
    for c in s:
        count += 1
    return count

fn repeat_char(s: str, times: int) -> str:
    mut result = ""
    mut i = 0
    while i < times:
        result = result + s
        i += 1
    return result

# ── list ──────────────────────────────────────────────────────────────────────

fn list_sum(items: list) -> int:
    mut total = 0
    for v in items:
        total += v
    return total

fn list_max(items: list) -> int:
    mut best = items[0]
    mut i = 1
    while i < len(items):
        let v = items[i]
        if v > best:
            best = v
        i += 1
    return best

fn list_count_if(items: list, threshold: int) -> int:
    mut count = 0
    for v in items:
        if v > threshold:
            count += 1
    return count

# ── class instances ───────────────────────────────────────────────────────────

fn sum_vec3_lengths(vecs: list) -> float:
    mut total = 0.0
    for v in vecs:
        total = total + vec3_len_sq(v)
    return total

fn find_max_pair(pairs: list) -> int:
    mut best = pairs[0].val
    for p in pairs:
        let v = p.val
        if v > best:
            best = v
    return best

fn collect_keys(pairs: list) -> str:
    mut result = ""
    for p in pairs:
        result = result + p.key
    return result

# ── mixed: list of instances + arithmetic ────────────────────────────────────

fn weighted_sum(vecs: list, weights: list, n: int) -> float:
    mut total = 0.0
    mut i = 0
    while i < n:
        let v = vecs[i]
        let w = weights[i]
        total = total + vec3_len_sq(v) * w
        i += 1
    return total

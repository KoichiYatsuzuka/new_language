# async_bench.tl — sequential vs parallel async comparison
#
# Each "task" counts primes up to LIMIT (pure integer loop, CPU-bound).
# We run TASKS tasks sequentially, then the same TASKS tasks in parallel
# using AsyncManager with NUM_THREADS threads, and compare elapsed times.

import[py-int] time

const LIMIT: int = 50_000
const TASKS: int = 8
const NUM_THREADS: int = 4

fn count_primes(n: int) -> int:
    if n < 2:
        return 0
    mut count = 0
    mut i = 2
    while i <= n:
        mut is_prime = True
        mut j = 2
        while j * j <= i:
            if i % j == 0:
                is_prime = False
            j += 1
        if is_prime:
            count += 1
        i += 1
    return count

# ── sequential ──────────────────────────────────────────────────────────────
print("=== sequential ===")
let t0 = time.time()

mut k = 0
mut seq_result = 0
while k < TASKS:
    seq_result = count_primes(LIMIT)
    k += 1

let t1 = time.time()
let seq_elapsed = t1 - t0
print("tasks:", TASKS, "  result:", seq_result)
print("sequential elapsed:", seq_elapsed, "s")

# ── async (parallel) ────────────────────────────────────────────────────────
print("")
print("=== async (", NUM_THREADS, "threads ) ===")
let t2 = time.time()

mut mng = AsyncManager(num_thread=NUM_THREADS)
mut m = 0
while m < TASKS:
    mng <- async->int:
        block_return count_primes(LIMIT)
    m += 1

mng.wait_for_finish()

let t3 = time.time()
let par_elapsed = t3 - t2
print("tasks:", TASKS, "  result[0]:", mng.results[0])
print("async elapsed:     ", par_elapsed, "s")

# ── speedup ─────────────────────────────────────────────────────────────────
print("")
let speedup = seq_elapsed / par_elapsed
print("speedup: ", speedup, "x")
print("(ideal with", NUM_THREADS, "threads:", NUM_THREADS, "x)")

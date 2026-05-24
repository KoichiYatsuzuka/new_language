# async_demo.tl — demonstrates the async system
#
# AsyncManager(num_thread=N) manages a pool of OS threads.
# mng <- async->T: body  submits a task; the body runs in a thread.
# All variables captured at submission time are deep-cloned.

fn slow_add(a: int, b: int) -> int:
    mut i = 0
    while i < 5_000_000:
        i += 1
    return a + b

# ── basic usage ──────────────────────────────────────────────────────────────
print("=== basic usage ===")

mut mng = AsyncManager(num_thread=3)

let x = 10
let y = 20

mng <- async->int:
    block_return slow_add(x, y)

mng <- async->int:
    block_return slow_add(100, 200)

mng <- async->int:
    block_return slow_add(1, 2)

mng.wait_for_finish()

print("results:", mng.results)          # [30, 300, 3]
print("errors:", mng.error_list)        # [None, None, None]
print("all_done:", mng.all_done())      # True
print("progress:", mng.progress_status) # [Async.Done, Async.Done, Async.Done]

# ── raise_immediately on first error ─────────────────────────────────────────
print("")
print("=== raise_immediately (error propagated via try/except) ===")

mut mng2 = AsyncManager(num_thread=2, raise_immediately=True)

mng2 <- async->int:
    block_return 42

mng2 <- async->int:
    raise RuntimeError("SimulatedError: something went wrong")

mng2 <- async->int:
    block_return 99

try:
    mng2.wait_for_finish()
except:
    print("caught: an async task raised an error")

print("error_list:", mng2.error_list)

# ── raise_immediately=False: collect all errors manually ──────────────────────
print("")
print("=== raise_immediately=False (check errors afterward) ===")

mut mng3 = AsyncManager(num_thread=2)

mng3 <- async->int:
    block_return slow_add(1, 1)

mng3 <- async->int:
    raise RuntimeError("AnotherError: oops")

mng3.wait_for_finish()
print("results:", mng3.results)
print("error_list:", mng3.error_list)

# ── Async status enum ─────────────────────────────────────────────────────────
print("")
print("=== Async status enum ===")
print(Async.Waiting)   # Async.Waiting
print(Async.Running)   # Async.Running
print(Async.Done)      # Async.Done

# try / except / finally — basic and internal-error examples

# ── 1. User-raised exception ──────────────────────────────────────────────────
print("=== user raise ===")
try:
    raise ValueError("bad input")
except ValueError as e:
    print("caught:", e.message)

# ── 2. Internal TypeError (type mismatch) ─────────────────────────────────────
print("=== TypeError ===")
mut result = ""
try:
    let x = "hello" + 42
except TypeError as e:
    result = "caught TypeError: " + e.message
print(result)

# ── 3. Internal IndexError ────────────────────────────────────────────────────
print("=== IndexError ===")
let lst = [10, 20, 30]
try:
    let y = lst[99]
except IndexError as e:
    print("caught IndexError:", e.message)

# ── 4. Internal KeyError ──────────────────────────────────────────────────────
print("=== KeyError ===")
let d = {"a": 1, "b": 2}
try:
    let z = d["missing"]
except KeyError as e:
    print("caught KeyError:", e.message)

# ── 5. ZeroDivisionError ──────────────────────────────────────────────────────
print("=== ZeroDivisionError ===")
try:
    let q = 10 / 0
except ZeroDivisionError as e:
    print("caught ZeroDivisionError:", e.message)

# ── 6. NameError ──────────────────────────────────────────────────────────────
print("=== NameError ===")
try:
    let v = no_such_variable
except NameError as e:
    print("caught NameError:", e.message)

# ── 7. Internal error propagating through a function call ────────────────────
print("=== through function ===")
fn risky_div(a: int, b: int) -> int:
    return a / b

try:
    risky_div(5, 0)
except ZeroDivisionError as e:
    print("caught from function:", e.message)

# ── 8. Bare except catches any internal error ─────────────────────────────────
print("=== bare except ===")
try:
    let w = lst[100]
except:
    print("bare except fired")

# ── 9. finally always runs ────────────────────────────────────────────────────
print("=== finally ===")
mut fin = "not run"
try:
    let _ = 1 / 0
except ZeroDivisionError:
    print("handler ran")
finally:
    fin = "ran"
print("finally:", fin)

# ── 10. Uncaught internal error propagates as a proper traceback ──────────────
print("=== propagation (no handler) — next line raises ===")
try:
    try:
        let _ = lst[50]
    except ValueError:
        print("wrong handler — should not print")
except IndexError as e:
    print("outer caught IndexError:", e.message)

print("done")

# collection.tl — list, dict, and tuple

# ============================================================
# 1. List
# ============================================================
print("=== list ===")

let nums = [10, 20, 30, 40, 50]
print(nums)
print("len:", len(nums))

# Iteration
mut sum = 0
for n in nums:
    sum += n
print("sum:", sum)   # 150

# Manual iterator
let it = nums.__iter__()
print(it.next())   # 10
print(it.next())   # 20

# List of strings
let fruits = ["apple", "banana", "cherry"]
for f in fruits:
    print("-", f)

# Mutable list
mut scores = [5, 3, 8, 1]
print("scores:", scores)

# ============================================================
# 2. Dict
# ============================================================
print("=== dict ===")

# Literal dict
let capitals = {"Japan": "Tokyo", "France": "Paris", "USA": "Washington"}
print(capitals["Japan"])    # Tokyo
print(capitals["France"])   # Paris

# Mutable dict — add and update entries
mut stock = {"apple": 10, "banana": 5}
stock["cherry"] = 3
stock["apple"] = 12
print(stock["apple"])   # 12
print(stock["cherry"])  # 3

# Typed empty dict
mut counts = dict[str, int]()
counts["a"] = 1
counts["b"] = 2
counts["a"] += 10
print(counts["a"])   # 11
print(counts["b"])   # 2

# Nested dict
let nested = {"outer": {"inner": 42}}
print(nested["outer"]["inner"])   # 42

# ============================================================
# 3. Tuple
# ============================================================
print("=== tuple ===")

# Empty tuple
let empty = ()
print(empty)

# Single-element tuple (trailing comma required)
let single = (99,)
print(single)

# Multi-element tuple
let triple = (1, "hello", True)
print(triple)

let pair = (10, 20)
print(pair)

# Tuple equality
let u = (1, 2, 3)
let v = (1, 2, 3)
let w = (1, 2, 4)
print(u == v)   # True
print(u == w)   # False

# Tuple truthiness (non-empty = truthy, empty = falsy)
print(not ())        # True  (empty tuple is falsy)
print(not (1, 2))    # False (non-empty is truthy)

# Tuples as return values
fn minmax(a: int, b: int) -> Any:
    if a < b:
        return (a, b)
    return (b, a)

let mm = minmax(7, 3)
print(mm)   # (3, 7)

# Nested tuples
let nested_t = ((1, 2), (3, 4))
print(nested_t)   # ((1, 2), (3, 4))

# ============================================================
# 4. Mixed usage
# ============================================================
print("=== mixed ===")

# List of tuples (list is iterable; tuples printed as-is)
let points = [(0, 0), (1, 2), (3, 4)]
for p in points:
    print(p)

# Dict with counted values
mut groups = dict[str, int]()
groups["evens"] = 0
groups["odds"]  = 0
for i in range(10):
    if i % 2 == 0:
        groups["evens"] += 1
    else:
        groups["odds"] += 1
print("evens:", groups["evens"])   # 5
print("odds:",  groups["odds"])    # 5

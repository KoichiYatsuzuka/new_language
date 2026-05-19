# collection.tl — list, dict, tuple, and set

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
# 4. Set
# ============================================================
print("=== set ===")

# Set literal — automatically deduplicated
let s = {1, 2, 2, 3, 1}
print(s)           # {1, 2, 3}
print(len(s))      # 3

# Empty set via constructor (not {}, which is an empty dict)
let e = set()
print(e)           # set()
print(bool(e))     # False

# set() from list / string
let from_list = set([4, 5, 4, 6])
print(from_list)   # {4, 5, 6}
let chars = set("hello")
print(len(chars))  # 4  (h, e, l, o)

# add / discard / remove
let a = {10, 20, 30}
a.add(40)
print(a)          # {10, 20, 30, 40}
a.discard(20)
print(a)          # {10, 30, 40}
a.discard(99)     # no error when element is absent
a.remove(30)
print(a)          # {10, 40}

# pop / clear
let b = {7, 8, 9}
let popped = b.pop()
print(len(b))     # 2
let c = {1, 2, 3}
c.clear()
print(c)          # set()

# in / not in
print(2 in {1, 2, 3})        # True
print(99 not in {1, 2, 3})   # True

# Set operators: | & - ^
let x = {1, 2, 3}
let y = {2, 3, 4}
print(x | y)   # {1, 2, 3, 4}  union
print(x & y)   # {2, 3}        intersection
print(x - y)   # {1}           difference
print(x ^ y)   # {1, 4}        symmetric difference

# Method equivalents
print(x.union(y))                 # {1, 2, 3, 4}
print(x.intersection(y))         # {2, 3}
print(x.difference(y))           # {1}
print(x.symmetric_difference(y)) # {1, 4}

# Equality
print({1, 2, 3} == {3, 2, 1})   # True
print({1, 2} == {1, 2, 3})      # False

# issubset / issuperset
let small = {1, 2}
let big   = {1, 2, 3}
print(small.issubset(big))    # True
print(big.issuperset(small))  # True
print(big.issubset(small))    # False

# copy
let orig = {1, 2, 3}
let cp = orig.copy()
cp.add(4)
print(orig)   # {1, 2, 3}  (unaffected)
print(cp)     # {1, 2, 3, 4}

# Iteration
mut total = 0
for n in {1, 2, 3, 4, 5}:
    total = total + n
print(total)   # 15

# ============================================================
# 5. Mixed usage
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

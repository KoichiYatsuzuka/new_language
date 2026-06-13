# collection_error.tl — intentional collection errors
#
# Each section raises a different error.
# Only one should be active at a time.

# --- Error 1: KeyError — accessing a missing dict key ---
# let d = {"a": 1, "b": 2}
# print(d["c"])   # KeyError: key 'c' not found

# --- Error 2: KeyError — set.remove() on an absent element ---
let s = {1, 2, 3}
s.remove(99)   # KeyError: 99 is not in set

# --- Error 3: StaticTypeError — tuple unpack missing qualifier ---
# Each named variable in a tuple unpack requires `let` or `mut`.
# let x, y = (1, 2)   # StaticTypeError: variable 'y' requires `let` or `mut` qualifier

# --- Error 4: IndexError — accessing an out-of-range list index ---
# let xs = [1, 2, 3]
# let r = xs[10]   # IndexError: list index 10 out of range

# --- Error 5: TypeError — raw int used as slice bound (must wrap in Index()) ---
# mut lst = [1, 2, 3, 4, 5]
# print(lst[1:3])   # TypeError: slice begin must be Index or None

# --- Error 6: StaticTypeError — typed collection argument mismatch ---
# fn sum_ints(items: list[int]) -> int:
#     mut total = 0
#     for x in items:
#         total += x
#     return total
# let words: list[str] = ["hello", "world"]
# sum_ints(words)   # StaticTypeError: list[str] is not compatible with list[int]

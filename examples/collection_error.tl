# collection_error.tl — intentional collection errors
#
# Each section raises a different runtime error.
# Only one should be active at a time.

# --- Error 1: KeyError — accessing a missing dict key ---
# let d = {"a": 1, "b": 2}
# print(d["c"])   # KeyError: key 'c' not found

# --- Error 2: KeyError — set.remove() on an absent element ---
let s = {1, 2, 3}
s.remove(99)   # KeyError: 99 is not in set

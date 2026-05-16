# collection_error.tl — KeyError: accessing a missing dict key
#
# Expected error (runtime KeyError):
#   Subscripting a dict with a key that does not exist raises a KeyError.

let d = {"a": 1, "b": 2}
print(d["a"])   # 1 — OK
print(d["c"])   # KeyError: key 'c' not found

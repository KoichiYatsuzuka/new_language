## ── Read slicing ────────────────────────────────────────────────────────────

mut lst = [10, 20, 30, 40, 50]

# basic bounds
print(lst[Index(1):Index(4)])    # [20, 30, 40]
print(lst[:Index(3)])            # [10, 20, 30]
print(lst[Index(2):])            # [30, 40, 50]
print(lst[:])                    # [10, 20, 30, 40, 50]

# step
print(lst[::2])                  # [10, 30, 50]
print(lst[Index(1):Index(4):2])  # [20, 40]
print(lst[::-1])                 # [50, 40, 30, 20, 10]

# string slicing
let word = "hello"
print(word[Index(1):Index(4)])   # ell
print(word[::-1])                # olleh

# tuple slicing
let t = (1, 2, 3, 4, 5)
print(t[Index(1):Index(3)])      # (2, 3)
print(t[::-1])                   # (5, 4, 3, 2, 1)

## ── slice() constructor ─────────────────────────────────────────────────────

let s = slice(Index(1), Index(4))
print(s)                         # slice(<Index object>, <Index object>, None)
print(lst[s])                    # [20, 30, 40]

let s2 = slice(Index(0), Index(5), 2)
print(lst[s2])                   # [10, 30, 50]

# attribute access
print(s.step)                    # None

## ── List slice assignment (step = 1) ────────────────────────────────────────

mut a = [1, 2, 3, 4, 5]

# replace same length
a[Index(1):Index(3)] = [20, 30]
print(a)                         # [1, 20, 30, 4, 5]

# replace with fewer elements (shrink)
a[Index(1):Index(3)] = [99]
print(a)                         # [1, 99, 4, 5]

# replace with more elements (grow)
a[Index(1):Index(2)] = [10, 20, 30]
print(a)                         # [1, 10, 20, 30, 4, 5]

# delete a slice (assign empty list)
a[Index(1):Index(4)] = []
print(a)                         # [1, 4, 5]

# prepend via empty-range insertion
a[:Index(0)] = [0]
print(a)                         # [0, 1, 4, 5]

# append via empty-range insertion at end
a[Index(4):] = [6, 7]
print(a)                         # [0, 1, 4, 5, 6, 7]

## ── List slice assignment (step ≠ 1) ────────────────────────────────────────

mut b = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

# every other element
b[::2] = [10, 30, 50, 70, 90]
print(b)                         # [10, 1, 30, 3, 50, 5, 70, 7, 90, 9]

# reverse-order assignment
b[::-1] = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
print(b)                         # [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

## ── Assign from tuple / string ──────────────────────────────────────────────

mut c = [1, 2, 3, 4, 5]
c[Index(1):Index(3)] = (20, 30)   # tuple RHS
print(c)                          # [1, 20, 30, 4, 5]

c[Index(0):Index(3)] = "abc"      # string RHS (each char becomes an element)
print(c)                          # ['a', 'b', 'c', 4, 5]

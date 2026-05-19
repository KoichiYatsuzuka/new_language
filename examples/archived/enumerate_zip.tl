# enumerate and zip built-ins

# ============================================================
# enumerate(iterable [, start=0])
# Returns an iterator of (index, value) tuples.
# ============================================================

let fruits = ["apple", "banana", "cherry"]

# Basic usage — index starts at 0
for i, name in enumerate(fruits):
    print(i, name)
# 0 apple
# 1 banana
# 2 cherry

print("---")

# start= keyword shifts the index
for i, name in enumerate(fruits, start=1):
    print(i, name)
# 1 apple
# 2 banana
# 3 cherry

print("---")

# enumerate over a tuple
let coords = (10, 20, 30)
for i, v in enumerate(coords):
    print(i, v)
# 0 10
# 1 20
# 2 30

print("---")

# ============================================================
# zip(iter1, iter2, ...)
# Returns an iterator of tuples; stops at the shortest iterable.
# ============================================================

let names = ["Alice", "Bob", "Carol"]
let scores = [95, 82, 78]

for name, score in zip(names, scores):
    print(name, score)
# Alice 95
# Bob 82
# Carol 78

print("---")

# Stops at the shortest input
let short = [1, 2]
let long = [10, 20, 30, 40]
for a, b in zip(short, long):
    print(a, b)
# 1 10
# 2 20

print("---")

# Three iterables at once
let xs = [1, 2, 3]
let ys = [4, 5, 6]
let zs = [7, 8, 9]
for x, y, z in zip(xs, ys, zs):
    print(x + y + z)
# 12
# 15
# 18

print("---")

# Combining enumerate and zip
let a = [10, 20, 30]
let b = [40, 50, 60]
for i, pair in enumerate(zip(a, b)):
    let x, let y = pair
    print(i, x, y)
# 0 10 40
# 1 20 50
# 2 30 60

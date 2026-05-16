# block_scope.tl — block scoping, variable lifetimes, outer-variable access

# --- block: statement creates a new scope ---
print("=== block scope ===")

mut outer = 10

block:
    mut inner = 99
    outer = 42            # outer-scope variable can be modified
    print("inside block: outer =", outer)   # 42
    print("inside block: inner =", inner)   # 99

print("after block: outer =", outer)        # 42
# inner is no longer visible here (NameError if accessed)

# --- if blocks scope their declared variables ---
print("=== if scope ===")

mut result = 0

if True:
    mut local = 5
    result = local * 2
    print("inside if: local =", local)  # 5

print("result after if:", result)  # 10
# local is out of scope here

# --- for loop body is also its own scope ---
print("=== for scope ===")

mut sum = 0

for i in range(5):
    mut step = i * 2      # scoped to this iteration's block
    sum += step

print("sum:", sum)   # 0+2+4+6+8 = 20
# step is out of scope here

# --- while loop scope ---
print("=== while scope ===")

mut total = 0
mut idx = 0

while idx < 4:
    mut contribution = idx * idx
    total += contribution
    idx += 1

print("total:", total)   # 0+1+4+9 = 14

# --- nested blocks and shadowing ---
print("=== nested blocks ===")

mut val = 1

block:
    mut val2 = 100
    block:
        mut val3 = val2 + val
        print("inner-inner val3:", val3)   # 101
    # val3 is out of scope here
    print("inner val2:", val2)             # 100

print("outer val:", val)    # 1

# --- block lets you create a temporary computation area ---
print("=== block as computation scope ===")

mut answer = 0

block:
    let a = 7
    let b = 6
    answer = a * b

print("answer:", answer)   # 42

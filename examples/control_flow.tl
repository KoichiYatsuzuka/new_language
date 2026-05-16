# control_flow.tl — if/elif/else, while, for, break, continue, pass

# ============================================================
# 1. if / elif / else
# ============================================================
print("=== if / elif / else ===")

fn grade(score: int) -> str:
    if score >= 90:
        return "A"
    elif score >= 80:
        return "B"
    elif score >= 70:
        return "C"
    elif score >= 60:
        return "D"
    else:
        return "F"

print(grade(95))   # A
print(grade(83))   # B
print(grade(71))   # C
print(grade(62))   # D
print(grade(40))   # F

# Nested if
let x = 15
if x > 0:
    if x % 2 == 0:
        print("positive even")
    else:
        print("positive odd")   # positive odd
else:
    print("non-positive")

# Logical operators in condition
let a = 5
let b = 10
if a > 0 and b > 0:
    print("both positive")   # both positive

if a > 100 or b > 5:
    print("at least one big")   # at least one big

if not (a == b):
    print("a and b are different")   # a and b are different

# ============================================================
# 2. while loop
# ============================================================
print("=== while ===")

# Basic accumulation
mut i = 1
mut sum_odds = 0
while i <= 9:
    if i % 2 != 0:
        sum_odds += i
    i += 1
print("sum of odd 1-9:", sum_odds)   # 25

# while True with break
mut countdown = 5
while True:
    if countdown == 0:
        break
    print("T-", countdown)
    countdown -= 1
print("liftoff!")

# continue to skip iterations
mut evens_only = 0
mut j = 0
while j < 10:
    j += 1
    if j % 2 != 0:
        continue
    evens_only += j
print("sum of even 2-10:", evens_only)   # 30

# ============================================================
# 3. for loop
# ============================================================
print("=== for ===")

# range with start/stop
mut total = 0
for n in range(1, 6):
    total += n
print("sum 1-5:", total)   # 15

# range with step
print("even 0-8:")
for n in range(0, 10, 2):
    print(" ", n)

# Iterate list literals
for fruit in ["apple", "banana", "cherry"]:
    print("-", fruit)

# break inside for
mut found = -1
for n in range(10):
    if n * n > 30:
        found = n
        break
print("first n where n*n > 30:", found)   # 6  (6*6=36)

# continue inside for
mut skip_sum = 0
for n in range(1, 11):
    if n % 3 == 0:
        continue
    skip_sum += n
print("sum 1-10 skipping multiples of 3:", skip_sum)   # 1+2+4+5+7+8+10 = 37

# ============================================================
# 4. Nested loops
# ============================================================
print("=== nested loops ===")

# Multiplication table (3x3)
for row in range(1, 4):
    for col in range(1, 4):
        print(row * col, " ")

# break only exits the inner loop
mut outer_ran = 0
for outer in range(3):
    outer_ran += 1
    for inner in range(10):
        if inner == 2:
            break
print("outer ran:", outer_ran)   # 3

# ============================================================
# 5. pass (no-op placeholder)
# ============================================================
print("=== pass ===")

for _ in range(5):
    pass   # deliberate no-op

fn placeholder() -> None:
    pass

placeholder()
print("pass examples done")

# ============================================================
# 6. for over string and custom iterator
# ============================================================
print("=== for over string ===")

mut char_count = 0
for ch in "hello":
    char_count += 1
print("len of 'hello':", char_count)   # 5

# Custom class with gen __iter__
class Countdown:
    mut start: int

    gen __iter__(self) -> int:
        mut n = self.start
        while n >= 0:
            yield n
            n -= 1

for v in Countdown(3):
    print(v)   # 3 2 1 0

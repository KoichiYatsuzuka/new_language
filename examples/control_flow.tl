# control_flow.tl — if/elif/else, while, for, break, continue, pass, match, block expressions, block scoping

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

# ============================================================
# 7. match statement
# ============================================================
print("=== match: value comparison ===")

let score = 85
let grade = block:
    match (score // 10):
        case 10:
            block_return "A+"
        case 9:
            block_return "A"
        case 8:
            block_return "B"
        case 7:
            block_return "C"
        case _:
            block_return "F"
    block_return "F"
print(grade)   # B

let s = "hello"
match (s):
    case "world":
        print("world")
    case "hello":
        print("hello")   # hello
    case _:
        print("unknown")

# wildcard only
let n = 999
match (n):
    case _:
        print("caught by wildcard")   # caught by wildcard

# no arm matches — does nothing
let y = 7
match (y):
    case 1:
        print("one")
    case 2:
        print("two")

# block_return inside match exits the enclosing block: early
mut result = 0
block:
    match (1):
        case 1:
            result = 42
            block_return 0
    result = 999
print(result)   # 42

print("=== match: type-pattern arms ===")

let v = 42
match (v):
    is int:
        print("int")   # int
    is str:
        print("str")
    is float:
        print("float")

class Dog:
    fn bark(self) -> str:
        return "woof"

class Cat:
    fn meow(self) -> str:
        return "meow"

mut animal = Dog()
match (animal):
    is Dog:
        print("it's a dog")   # it's a dog
    is Cat:
        print("it's a cat")

# ============================================================
# 8. Control flow as expressions (block_return / loop_yield)
# ============================================================
print("=== if expression ===")

mut cf_flag = True
let if_expr_x = if cf_flag == True ->str:
    block_return "true"
else:
    block_return "false"
print(if_expr_x)   # true

cf_flag = False
let if_expr_y = if cf_flag == True ->str:
    block_return "true"
else:
    block_return "false"
print(if_expr_y)   # false

# if without else returns None when no branch matches
let nothing = if False ->str:
    block_return "unreachable"
print(nothing)   # None

print("=== for expression ===")

let evens = for i in range(10) ->list[int]:
    if i % 2 == 0:
        loop_yield i
print(evens)   # [0, 2, 4, 6, 8]

let squares = for i in range(1, 6) ->list[int]:
    loop_yield i * i
print(squares)   # [1, 4, 9, 16, 25]

# break exits the loop and returns the accumulated list
let partial = for i in range(10) ->list[int]:
    if i == 3:
        break
    loop_yield i
print(partial)   # [0, 1, 2]

print("=== while expression ===")

mut cf_count = 0
let while_result = while cf_count < 5 ->list[int]:
    loop_yield cf_count
    cf_count += 1
print(while_result)   # [0, 1, 2, 3, 4]

print("=== match expression ===")

let match_score = 85
let match_grade = match (match_score // 10) ->str:
    case 10:
        block_return "A+"
    case 9:
        block_return "A"
    case 8:
        block_return "B"
    case 7:
        block_return "C"
    case _:
        block_return "F"
print(match_grade)   # B

# break in regular for loop
mut found_val = -1
for i in range(10):
    if i == 5:
        found_val = i
        break
    found_val = i
print(found_val)   # 5

print("=== block: expression ===")

let computed = block ->int:
    let ba = 6
    let bb = 7
    block_return ba * bb
print(computed)   # 42

# loop_yield with no elements → None
let empty = for i in range(0) ->list[int]:
    loop_yield i
print(empty)   # None

print("=== ->Type annotation ===")

# Option[T]: None is accepted alongside the concrete type
let opt = block ->Option[int]:
    block_return None
print(opt)   # None

let opt2 = block ->Option[int]:
    block_return 7
print(opt2)   # 7

let maybe = if False ->Option[str]:
    block_return "value"
else:
    block_return None
print(maybe)   # None

# no annotation: no type check applied
let raw_block = block:
    block_return 99
print(raw_block)   # 99

# ============================================================
# 9. Block scoping
# ============================================================
print("=== block scope ===")

mut outer = 10

block:
    mut inner = 99
    outer = 42
    print("inside block: outer =", outer)   # 42
    print("inside block: inner =", inner)   # 99

print("after block: outer =", outer)        # 42

print("=== if scope ===")

mut scope_result = 0

if True:
    mut local = 5
    scope_result = local * 2
    print("inside if: local =", local)  # 5

print("result after if:", scope_result)  # 10

print("=== for scope ===")

mut scope_sum = 0

for si in range(5):
    mut step = si * 2
    scope_sum += step

print("sum:", scope_sum)   # 20

print("=== while scope ===")

mut scope_total = 0
mut scope_idx = 0

while scope_idx < 4:
    mut contribution = scope_idx * scope_idx
    scope_total += contribution
    scope_idx += 1

print("total:", scope_total)   # 14

print("=== nested blocks ===")

mut val = 1

block:
    mut val2 = 100
    block:
        mut val3 = val2 + val
        print("inner-inner val3:", val3)   # 101
    print("inner val2:", val2)             # 100

print("outer val:", val)    # 1

print("=== block as computation scope ===")

mut answer = 0

block:
    let ba2 = 7
    let bb2 = 6
    answer = ba2 * bb2

print("answer:", answer)   # 42

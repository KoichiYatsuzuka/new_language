# control_flow_expr.tl — if/for/while/match as expressions with ->Type annotation

# --- if as expression with block_return ---

mut flag = True
let x = if flag == True ->str:
    block_return "true"
else:
    block_return "false"
print(x)   # true

flag = False
let y = if flag == True ->str:
    block_return "true"
else:
    block_return "false"
print(y)   # false

# --- if without else (returns None when no branch matches) ---

let nothing = if False ->str:
    block_return "unreachable"
print(nothing)   # None

# --- for as expression with loop_yield (list accumulation) ---

let evens = for i in range(10) ->list[int]:
    if i % 2 == 0:
        loop_yield i
print(evens)   # [0, 2, 4, 6, 8]

let squares = for i in range(1, 6) ->list[int]:
    loop_yield i * i
print(squares)   # [1, 4, 9, 16, 25]

# --- for as expression with block_return (early exit, single value) ---

let first_even = for i in range(1, 10) ->int:
    if i % 2 == 0:
        block_return i
print(first_even)   # 2

# --- break exits for expression loop (returns accumulated list so far) ---

let partial = for i in range(10) ->list[int]:
    if i == 3:
        break
    loop_yield i
print(partial)   # [0, 1, 2]

# --- while as expression with loop_yield ---

mut count = 0
let countdown = while count < 5 ->list[int]:
    loop_yield count
    count += 1
print(countdown)   # [0, 1, 2, 3, 4]

# --- while as expression with block_return ---

mut n = 0
let found = while n < 100 ->int:
    n += 1
    if n * n > 50:
        block_return n
print(found)   # 8  (8*8=64 > 50)

# --- match as expression with block_return ---

let score = 85
let grade = match (score // 10) ->str:
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
print(grade)   # B

# --- break as alias of block_return None in regular for loop ---

mut found_val = -1
for i in range(10):
    if i == 5:
        found_val = i
        break
    found_val = i
print(found_val)   # 5

# --- block_return inside nested if inside for expression ---

let result = for i in range(10) ->int:
    if i > 4:
        block_return i
print(result)   # 5

# --- block: expression (existing, still works) ---

let computed = block ->int:
    let a = 6
    let b = 7
    block_return a * b
print(computed)   # 42

# --- loop_yield in for expression with no elements yields empty list → None ---

let empty = for i in range(0) ->list[int]:
    loop_yield i
print(empty)   # None

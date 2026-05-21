# block_return_type_check.tl — runtime type checking for block_return
#
# The ->Type annotation on block:/if/for/while/match expressions is checked at runtime.
# Note: for loop_yield, the parser strips element types (list[int] -> "list"), so only
# the outer list type is enforced; element type checking requires a future parser enhancement.

# --- block: with ->int ---

let x = block ->int:
    block_return 42
print(x)   # 42

# --- if expression with ->str ---

let greeting = if True ->str:
    block_return "hello"
else:
    block_return "bye"
print(greeting)   # hello

# --- for expression: block_return against ->int (early exit) ---

let first_odd = for i in range(10) ->int:
    if i % 2 == 1:
        block_return i
print(first_odd)   # 1

# --- while expression: block_return against ->int ---

mut n = 0
let found = while n < 100 ->int:
    n += 1
    if n * n > 50:
        block_return n
print(found)   # 8

# --- match expression with ->str ---

let score = 90
let grade = match (score // 10) ->str:
    case 10:
        block_return "A+"
    case 9:
        block_return "A"
    case _:
        block_return "other"
print(grade)   # A

# --- Option[int]: None is accepted ---

let opt = block ->Option[int]:
    block_return None
print(opt)   # None

let opt2 = block ->Option[int]:
    block_return 7
print(opt2)   # 7

# --- Option[str]: None or str both accepted ---

let maybe = if False ->Option[str]:
    block_return "value"
else:
    block_return None
print(maybe)   # None

# --- no annotation: no type check, any value accepted ---

let raw = block:
    block_return 99
print(raw)   # 99

# --- loop_yield: list type annotation accepted ---

let squares = for i in range(1, 6) ->list[int]:
    loop_yield i * i
print(squares)   # [1, 4, 9, 16, 25]

# block_expr.tl — block: as an expression with block_return

# --- block_return: return a single value from a block expression ---

let x = block:
    let a = 6
    let b = 7
    block_return a * b
print(x)   # 42

# --- block_return inside conditional ---

mut n = 15
let desc = block:
    if n % 15 == 0:
        block_return "FizzBuzz"
    elif n % 3 == 0:
        block_return "Fizz"
    elif n % 5 == 0:
        block_return "Buzz"
    else:
        block_return str(n)
print(desc)   # FizzBuzz

# --- block with no block_return returns None ---

let nothing = block:
    mut tmp = 100
    tmp += 1
print(nothing)   # None

# --- block: expr with match ---

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

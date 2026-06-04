import copy
from typing import Generator

print('=== if / elif / else ===')
def grade(score: int) -> str:
    if (score >= 90):
        return 'A'
    elif (score >= 80):
        return 'B'
    elif (score >= 70):
        return 'C'
    elif (score >= 60):
        return 'D'
    else:
        return 'F'

print(grade(95))
print(grade(83))
print(grade(71))
print(grade(62))
print(grade(40))
x = 15
if (x > 0):
    if ((x % 2) == 0):
        print('positive even')
    else:
        print('positive odd')
else:
    print('non-positive')
a = 5
b = 10
if ((a > 0) and (b > 0)):
    print('both positive')
if ((a > 100) or (b > 5)):
    print('at least one big')
if (not (a == b)):
    print('a and b are different')
print('=== while ===')
i = 1
sum_odds = 0
while (i <= 9):
    if ((i % 2) != 0):
        sum_odds += i
    i += 1
print('sum of odd 1-9:', copy.deepcopy(sum_odds))
countdown = 5
while True:
    if (countdown == 0):
        break
    print('T-', copy.deepcopy(countdown))
    countdown -= 1
print('liftoff!')
evens_only = 0
j = 0
while (j < 10):
    j += 1
    if ((j % 2) != 0):
        continue
    evens_only += j
print('sum of even 2-10:', copy.deepcopy(evens_only))
print('=== for ===')
total = 0
for n in range(1, 6):
    total += n
print('sum 1-5:', copy.deepcopy(total))
print('even 0-8:')
for n in range(0, 10, 2):
    print(' ', n)
for fruit in ['apple', 'banana', 'cherry']:
    print('-', fruit)
found = (-1)
for n in range(10):
    if ((n * n) > 30):
        found = n
        break
print('first n where n*n > 30:', copy.deepcopy(found))
skip_sum = 0
for n in range(1, 11):
    if ((n % 3) == 0):
        continue
    skip_sum += n
print('sum 1-10 skipping multiples of 3:', copy.deepcopy(skip_sum))
print('=== nested loops ===')
for row in range(1, 4):
    for col in range(1, 4):
        print((row * col), ' ')
outer_ran = 0
for outer in range(3):
    outer_ran += 1
    for inner in range(10):
        if (inner == 2):
            break
print('outer ran:', copy.deepcopy(outer_ran))
print('=== pass ===')
for _ in range(5):
    pass
def placeholder() -> None:
    pass

placeholder()
print('pass examples done')
print('=== for over string ===')
char_count = 0
for ch in 'hello':
    char_count += 1
print('len of \'hello\':', copy.deepcopy(char_count))
class Countdown:
    start: int
    def __iter__(self) -> Generator[int, None, None]:
        n = self.start
        while (n >= 0):
            yield n
            n -= 1

    def __init__(self, start: int) -> None:
        self.start = start


for v in Countdown(3):
    print(v)
print('=== match: value comparison ===')
score = 85
grade = None
_sv0 = (score // 10)
if _sv0 == 10:
    grade = 'A+'
elif _sv0 == 9:
    grade = 'A'
elif _sv0 == 8:
    grade = 'B'
elif _sv0 == 7:
    grade = 'C'
else:
    grade = 'F'
grade = 'F'
print(grade)
s = 'hello'
_sv1 = s
if _sv1 == 'world':
    print('world')
elif _sv1 == 'hello':
    print('hello')
else:
    print('unknown')
n = 999
_sv2 = n
if True:  # wildcard
    print('caught by wildcard')
y = 7
_sv3 = y
if _sv3 == 1:
    print('one')
elif _sv3 == 2:
    print('two')
result = 0
_sv4 = 1
if _sv4 == 1:
    result = 42
    # block_return 0  # (no assignment target)
result = 999
print(copy.deepcopy(result))
print('=== match: type-pattern arms ===')
v = 42
_sv5 = v
if isinstance(_sv5, int):
    print('int')
elif isinstance(_sv5, str):
    print('str')
elif isinstance(_sv5, float):
    print('float')
class Dog:
    def bark(self) -> str:
        return 'woof'


class Cat:
    def meow(self) -> str:
        return 'meow'


animal = Dog()
_sv6 = animal
if isinstance(_sv6, Dog):
    print('it\'s a dog')
elif isinstance(_sv6, Cat):
    print('it\'s a cat')
print('=== if expression ===')
cf_flag = True
if_expr_x = None
if (cf_flag == True):
    if_expr_x = 'true'
else:
    if_expr_x = 'false'
print(if_expr_x)
cf_flag = False
if_expr_y = None
if (cf_flag == True):
    if_expr_y = 'true'
else:
    if_expr_y = 'false'
print(if_expr_y)
nothing = None
if False:
    nothing = 'unreachable'
print(nothing)
print('=== for expression ===')
evens = []
for i in range(10):
    if ((i % 2) == 0):
        evens.append(i)
print(evens)
squares = []
for i in range(1, 6):
    squares.append((i * i))
print(squares)
partial = []
for i in range(10):
    if (i == 3):
        break
    partial.append(i)
print(partial)
print('=== while expression ===')
cf_count = 0
while_result = []
while (cf_count < 5):
    while_result.append(cf_count)
    cf_count += 1
print(while_result)
print('=== match expression ===')
match_score = 85
match_grade = None
_sv7 = (match_score // 10)
if _sv7 == 10:
    match_grade = 'A+'
elif _sv7 == 9:
    match_grade = 'A'
elif _sv7 == 8:
    match_grade = 'B'
elif _sv7 == 7:
    match_grade = 'C'
else:
    match_grade = 'F'
print(match_grade)
found_val = (-1)
for i in range(10):
    if (i == 5):
        found_val = i
        break
    found_val = i
print(copy.deepcopy(found_val))
print('=== block: expression ===')
computed = None
ba = 6
bb = 7
computed = (ba * bb)
print(computed)
empty = []
for i in range(0):
    empty.append(i)
print(empty)
print('=== ->Type annotation ===')
opt = None
opt = None
print(opt)
opt2 = None
opt2 = 7
print(opt2)
maybe = None
if False:
    maybe = 'value'
else:
    maybe = None
print(maybe)
raw_block = None
raw_block = 99
print(raw_block)
print('=== block scope ===')
outer = 10
inner = 99
outer = 42
print('inside block: outer =', copy.deepcopy(outer))
print('inside block: inner =', copy.deepcopy(inner))
print('after block: outer =', copy.deepcopy(outer))
print('=== if scope ===')
scope_result = 0
if True:
    local = 5
    scope_result = (local * 2)
    print('inside if: local =', copy.deepcopy(local))
print('result after if:', copy.deepcopy(scope_result))
print('=== for scope ===')
scope_sum = 0
for si in range(5):
    step = (si * 2)
    scope_sum += step
print('sum:', copy.deepcopy(scope_sum))
print('=== while scope ===')
scope_total = 0
scope_idx = 0
while (scope_idx < 4):
    contribution = (scope_idx * scope_idx)
    scope_total += contribution
    scope_idx += 1
print('total:', copy.deepcopy(scope_total))
print('=== nested blocks ===')
val = 1
val2 = 100
val3 = (val2 + val)
print('inner-inner val3:', copy.deepcopy(val3))
print('inner val2:', copy.deepcopy(val2))
print('outer val:', copy.deepcopy(val))
print('=== block as computation scope ===')
answer = 0
ba2 = 7
bb2 = 6
answer = (ba2 * bb2)
print('answer:', copy.deepcopy(answer))

import copy
from typing import Any

print('=== list ===')
nums = [10, 20, 30, 40, 50]
print(nums)
print('len:', len(nums))
sum = 0
for n in nums:
    sum += n
print('sum:', copy.deepcopy(sum))
it = nums.__iter__()
print(it.next())
print(it.next())
fruits = ['apple', 'banana', 'cherry']
for f in fruits:
    print('-', f)
scores = [5, 3, 8, 1]
print('scores:', copy.deepcopy(scores))
print('=== dict ===')
capitals = {'Japan': 'Tokyo', 'France': 'Paris', 'USA': 'Washington'}
print(capitals['Japan'])
print(capitals['France'])
stock = {'apple': 10, 'banana': 5}
stock['cherry'] = 3
stock['apple'] = 12
print(stock['apple'])
print(stock['cherry'])
counts = dict[str, int]()
counts['a'] = 1
counts['b'] = 2
counts['a'] += 10
print(counts['a'])
print(counts['b'])
nested = {'outer': {'inner': 42}}
print(nested['outer']['inner'])
print('=== tuple ===')
empty = ()
print(empty)
single = (99,)
print(single)
triple = (1, 'hello', True,)
print(triple)
pair = (10, 20,)
print(pair)
u = (1, 2, 3,)
v = (1, 2, 3,)
w = (1, 2, 4,)
print((u == v))
print((u == w))
print((not ()))
print((not (1, 2,)))
def minmax(a: int, b: int) -> Any:
    if (a < b):
        return (a, b,)
    return (b, a,)

mm = minmax(7, 3)
print(mm)
nested_t = ((1, 2,), (3, 4,),)
print(nested_t)
tup1 = (1, 2,)
tx, ty = tup1
print(tx)
print(copy.deepcopy(ty))
tp, tq = (10, 20,)
print(tp)
print(tq)
ty = 99
print(copy.deepcopy(ty))
tm, tn = (3, 4,)
print(copy.deepcopy(tm))
print(tn)
tm = 100
print(copy.deepcopy(tm))
tup2 = (10, 20, 30, 40,)
tr, ts, _ = tup2
print(tr)
print(copy.deepcopy(ts))
tup3 = (5, 6,)
only, _ = tup3
print(only)
ti, tj = (7, 8,)
print(ti)
print(copy.deepcopy(tj))
print('=== set ===')
s = {1, 2, 2, 3, 1}
print(s)
print(len(s))
e = set()
print(e)
print(bool(e))
from_list = set([4, 5, 4, 6])
print(from_list)
chars = set('hello')
print(len(chars))
a = {10, 20, 30}
a.add(40)
print(a)
a.discard(20)
print(a)
a.discard(99)
a.remove(30)
print(a)
b = {7, 8, 9}
popped = b.pop()
print(len(b))
c = {1, 2, 3}
c.clear()
print(c)
print(2 in {1, 2, 3})
print(99 not in {1, 2, 3})
x = {1, 2, 3}
y = {2, 3, 4}
print((x | y))
print((x & y))
print((x - y))
print((x ^ y))
print(x.union(y))
print(x.intersection(y))
print(x.difference(y))
print(x.symmetric_difference(y))
print(({1, 2, 3} == {3, 2, 1}))
print(({1, 2} == {1, 2, 3}))
small = {1, 2}
big = {1, 2, 3}
print(small.issubset(big))
print(big.issuperset(small))
print(big.issubset(small))
orig = {1, 2, 3}
cp = orig.copy()
cp.add(4)
print(orig)
print(cp)
total = 0
for n in {1, 2, 3, 4, 5}:
    total = (total + n)
print(copy.deepcopy(total))
print('=== mixed ===')
points = [(0, 0,), (1, 2,), (3, 4,)]
for p in points:
    print(p)
groups = dict[str, int]()
groups['evens'] = 0
groups['odds'] = 0
for i in range(10):
    if ((i % 2) == 0):
        groups['evens'] += 1
    else:
        groups['odds'] += 1
print('evens:', groups['evens'])
print('odds:', groups['odds'])
print('=== subscript ===')
xs = [10, 20, 30, 40, 50]
print(xs[0])
print(xs[(-1)])
xs[2] = 99
print(xs[2])
greet = 'hello'
print(greet[0])
print(greet[(-1)])
idx_d = {'a': 1, 'b': 2}
print(idx_d['a'])
idx_d['c'] = 3
print(idx_d['c'])
idx_t = (100, 200, 300,)
print(idx_t[1])
print(idx_t[(-1)])
class Stack:
    def __init__(self) -> None:
        self.items = [0, 0, 0, 0, 0]

    def __getitem__(self, idx: int) -> int:
        return self.items[idx]

    def __setitem__(self, idx: int, val: int) -> None:
        self.items[idx] = val


stack = Stack()
stack[0] = 42
stack[1] = 7
print(stack[0])
print(stack[1])
print('=== slice ===')
lst = [10, 20, 30, 40, 50]
print(lst[Index(1):Index(4)])
print(lst[:Index(3)])
print(lst[Index(2):])
print(lst[:])
print(lst[::2])
print(lst[Index(1):Index(4):2])
print(lst[::(-1)])
word = 'hello'
print(word[Index(1):Index(4)])
print(word[::(-1)])
sl_t = (1, 2, 3, 4, 5,)
print(sl_t[Index(1):Index(3)])
print(sl_t[::(-1)])
slc = slice(Index(1), Index(4))
print(slc)
print(lst[slc])
slc2 = slice(Index(0), Index(5), 2)
print(lst[slc2])
print(slc.step)
sl_a = [1, 2, 3, 4, 5]
sl_a[Index(1):Index(3)] = [20, 30]
print(copy.deepcopy(sl_a))
sl_a[Index(1):Index(3)] = [99]
print(copy.deepcopy(sl_a))
sl_a[Index(1):Index(2)] = [10, 20, 30]
print(copy.deepcopy(sl_a))
sl_a[Index(1):Index(4)] = []
print(copy.deepcopy(sl_a))
sl_a[:Index(0)] = [0]
print(copy.deepcopy(sl_a))
sl_a[Index(4):] = [6, 7]
print(copy.deepcopy(sl_a))
sl_b = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
sl_b[::2] = [10, 30, 50, 70, 90]
print(copy.deepcopy(sl_b))
sl_b[::(-1)] = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]
print(copy.deepcopy(sl_b))
sl_c = [1, 2, 3, 4, 5]
sl_c[Index(1):Index(3)] = (20, 30,)
print(copy.deepcopy(sl_c))
sl_c[Index(0):Index(3)] = 'abc'
print(copy.deepcopy(sl_c))
print('=== typed generics ===')
def sum_ints(items: list[int]) -> int:
    total = 0
    for x in items:
        total += x
    return total

typed_nums = [1, 2, 3, 4, 5]
print('sum:', sum_ints(typed_nums))
def join_strs(items: list[str]) -> str:
    result = ''
    for s in items:
        result = ((result + s) + ' ')
    return result

words = ['hello', 'world']
print('joined:', join_strs(words))
def lookup(d: dict[str,int], key: str) -> int:
    return d[key]

scores = {'alice': 90, 'bob': 85}
print('alice:', lookup(copy.deepcopy(scores), 'alice'))
print('bob:', lookup(copy.deepcopy(scores), 'bob'))
def count_unique(items: set[int]) -> int:
    return len(items)

unique = {1, 2, 3, 2, 1}
print('unique count:', count_unique(unique))
def first_row(matrix: list[list[int]]) -> list[int]:
    for row in matrix:
        return row
    return []

matrix = [[1, 2], [3, 4]]
print('first row:', first_row(matrix))
class Stats:
    values: list[int]
    def max_val(self) -> int:
        m = 0
        for v in self.values:
            if (v > m):
                m = v
        return m

    def count(self) -> int:
        return len(self.values)

    def __init__(self, values: list[int]) -> None:
        self.values = values


stats = Stats(values=[3, 1, 4, 1, 5, 9, 2, 6])
print('count:', stats.count())
print('max:', stats.max_val())

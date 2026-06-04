import copy
from typing import Final, Callable

print('=== let ===')
name = 'Alice'
age = 30
pi = 3.14159
flag = True
print(name, age, pi, flag)
print('=== const ===')
MAX_ITEMS: Final = 100
LANG_NAME: Final = 'Havakyrie'
print(MAX_ITEMS, LANG_NAME)
print('=== mut ===')
score = 50
score = 85
print('score:', copy.deepcopy(score))
n = 10
n += 5
n -= 2
n *= 3
n //= 4
n %= 5
print('n after compound ops:', copy.deepcopy(n))
bits = 3
bits |= 12
bits &= 10
bits ^= 6
print('bits after bitwise ops:', copy.deepcopy(bits))
print('=== freeze ===')
temp = 42
print('before freeze:', copy.deepcopy(temp))
print('after freeze:', copy.deepcopy(temp))
class Config:
    debug: bool
    max_conn: int
    def __freeze__(self) -> None:
        print('Config locked: debug =', self.debug, 'max_conn =', self.max_conn)

    def __init__(self, debug: bool, max_conn: int) -> None:
        self.debug = debug
        self.max_conn = max_conn


cfg = Config(True, 10)
cfg.debug = False
cfg.debug = True
print('=== static mut ===')
def make_id_generator() -> Callable:
    next_id = 0  # static mut
    def next() -> int:
        next_id += 1
        return next_id

    return next

gen_a = make_id_generator()
gen_b = make_id_generator()
print(gen_a())
print(gen_a())
print(gen_b())
print('=== numeric literals ===')
decimal = 1000000
hex_val = 255
oct_val = 63
bin_val = 170
print(decimal)
print(hex_val)
print(oct_val)
print(bin_val)
print('=== strings ===')
s1 = 'double quoted'
s2 = 'single quoted'
s3 = 'triple\nquoted'
print(s1)
print(s2)
print(s3)

## py_iteration.tl — PyObject の for ループ反復例

import[py-int] py_calculator as calc

# Container は Python iterable; for ループで各要素を走査できる
let c = calc.make_container([10, 20, 30, 40])

mut total = 0
for x in c:
    total += x

print(total)   # 100

# ネストしたループも動作する
let outer = calc.make_container([1, 2, 3])
mut result = 0
for v in outer:
    result += v * 10

print(result)  # 60

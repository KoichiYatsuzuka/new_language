## py_binop.tl — PyObject の二項演算子例

import[py-int] py_calculator as calc

# lhs = PyObject: c * 3 → Container.__mul__
let c = calc.make_container([1, 2])
let c2 = c * 3
print(len(c2))    # 6

# rhs = PyObject: 3 * c → Container.__rmul__
let c3 = 3 * c
print(len(c3))    # 6

# PyObject + PyObject: Container.__add__
let a = calc.make_container([1, 2])
let b = calc.make_container([3, 4, 5])
let ab = a + b
print(len(ab))    # 5

# for ループで結果を確認する
mut total = 0
for v in ab:
    total += v
print(total)      # 15

##
## subscript.tl — __getitem__ / __setitem__ の使用例
##

# --- list ---
mut xs = [10, 20, 30, 40, 50]
print(xs[0])       # 10
print(xs[-1])      # 50

xs[2] = 99
print(xs[2])       # 99

# --- str ---
let s = "hello"
print(s[0])        # h
print(s[-1])       # o

# --- dict ---
mut d = {"a": 1, "b": 2}
print(d["a"])      # 1
d["c"] = 3
print(d["c"])      # 3

# --- tuple ---
let t = (100, 200, 300)
print(t[1])        # 200
print(t[-1])       # 300

# --- ユーザー定義クラス: __getitem__ / __setitem__ ---
class Stack:
    fn __init__(mut self) -> None:
        self.items = [0, 0, 0, 0, 0]

    fn __getitem__(self, let idx: int) -> int:
        return self.items[idx]

    fn __setitem__(mut self, let idx: int, let val: int) -> None:
        self.items[idx] = val

mut stack = Stack()
stack[0] = 42
stack[1] = 7
print(stack[0])    # 42
print(stack[1])    # 7

# --- import[py-int]: PyObject の __getitem__ / __setitem__ ---
import[py-int] py_calculator as calc

let c = calc.make_container([1, 2, 3])
print(c[0])        # 1

c[0] = 99
print(c[0])        # 99

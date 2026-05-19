# id() built-in function — returns a pointer identifying the object

# id() returns a pointer instance wrapping a uint value
let n = 42
let p = id(n)
print("id(42) =", str(p.value))

# Reference types: same variable → same id
class Box:
    fn __init__(mut self, let val: int) -> None:
        self.val = val

let a = Box(1)
let b = a            # let→let: same Rc, same id
let pa = id(a)
let pb = id(b)
print("id(a) == id(b):", str(pa.value == pb.value))  # True

# mut copy → new allocation → different id
mut c = a
let pc = id(c)
print("id(a) == id(c):", str(pa.value == pc.value))  # False

# Value types: equal values → equal ids (value-based identity)
let x = 0
let y = x            # let→let: no copy for value types either
let px = id(x)
let py = id(y)
print("id(x) == id(y) (int 0):", str(px.value == py.value))  # True

# uint type: id() value field is a uint
let u = uint(100)
let pu = id(u)
print("id(uint(100)) type is uint:", str(pu.value is uint))  # True

# pointer is a new_type wrapping uint
print("pointer.value of id(a):", str(pa.value))

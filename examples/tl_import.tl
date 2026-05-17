import tl_math as tm

let result: int = tm.add(3, 4)
print(result)

let msg: str = tm.greet("World")
print(msg)

print(tm.PI)

from tl_math import add, greet

let r2: int = add(10, 20)
print(r2)

let m2: str = greet("TL")
print(m2)

import geometry as geo

let a: float = geo.area_circle(5.0)
print(a)

let b: float = geo.area_rect(3.0, 4.0)
print(b)

print(geo.DIMS)

# cast.tl — demonstrates the => cast operator and __cast__ special method

# ── Basic cast: instance => primitive type ──────────────────────────────────

class Celsius:
    let value: float

    fn __init__(mut self, value: float)->None:
        self.value = value

    fn __cast__[float](let self)->float:
        return self.value

    fn __cast__[int](let self)->int:
        return int(self.value)

    fn __repr__(let self)->str:
        return "Celsius(" + repr(self.value) + ")"

let temp = Celsius(36.6)
let f: float = temp=>float
let i: int   = temp=>int
print(f)   # 36.6
print(i)   # 36

# ── Chaining: =>type followed by .method() ──────────────────────────────────

class JsonLike:
    let data: dict[str, int]

    fn __init__(mut self, data: dict[str, int])->None:
        self.data = data

    fn __cast__[dict[str, int]](let self)->dict[str, int]:
        return self.data

    fn __repr__(let self)->str:
        return "JsonLike(...)"

let d = dict[str, int]({"a": 1, "b": 2})
let jl = JsonLike(d)
let keys = jl=>dict[str, int].keys()
print(keys)  # ["a", "b"]

# ── new_type downcast: primitive => new_type ────────────────────────────────

new_type Meters: float
new_type Celsius2: float

let m = 5.0=>Meters       # equivalent to Meters(5.0)
let c2 = 100.0=>Celsius2

print(repr(m))   # Meters(5.0)
print(repr(c2))  # Celsius2(100.0)

new_type Count: int
let cnt = 42=>Count        # Count(42)
print(repr(cnt))           # Count(42)

# ── new_type upcast: new_type instance => base primitive ─────────────────────

let raw_f = m=>float       # Meters(5.0) => float  → 5.0
let raw_i = cnt=>int       # Count(42)   => int    → 42
print(raw_f)               # 5.0
print(raw_i)               # 42

# ── cross new_type cast: new_type instance => new_type with same base ─────────

new_type Kg: float
let kg = m=>Kg             # Meters(5.0) => Kg  — extracts inner value, wraps as Kg(5.0)
print(repr(kg))            # Kg(5.0)

# ── Automatic cast for `let` parameters ─────────────────────────────────────

class Wrapper:
    let val: int

    fn __init__(mut self, val: int)->None:
        self.val = val

    fn __cast__[int](let self)->int:
        return self.val

    fn __repr__(let self)->str:
        return "Wrapper(" + repr(self.val) + ")"

fn double(let x: int)->int:
    return x * 2

let w = Wrapper(21)
let result = double(w)   # auto-cast: w=>int = 21, then double(21) = 42
print(result)            # 42

# ── Multiple cast targets on the same class ──────────────────────────────────

class Point:
    let x: int
    let y: int

    fn __init__(mut self, x: int, y: int)->None:
        self.x = x
        self.y = y

    fn __cast__[str](let self)->str:
        return "(" + repr(self.x) + ", " + repr(self.y) + ")"

    fn __cast__[list](let self)->list:
        return [self.x, self.y]

    fn __repr__(let self)->str:
        return "Point(" + repr(self.x) + ", " + repr(self.y) + ")"

let p = Point(3, 4)
let s: str  = p=>str
let lst     = p=>list
print(s)    # (3, 4)
print(lst)  # [3, 4]

print("cast.tl OK")

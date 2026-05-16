# overload_success.tl
# 関数オーバーロードの動作確認サンプル（すべて正常終了するはず）

# ── 1. 引数の個数が異なるオーバーロード ──────────────────────────
fn greet() -> str:
    return "Hello!"

fn greet(name: str) -> str:
    return "Hello, " + name + "!"

fn greet(name: str, title: str) -> str:
    return "Hello, " + title + " " + name + "!"

print("=== 1. 引数の個数が異なるオーバーロード ===")
print(greet())
print(greet("Alice"))
print(greet("Smith", "Dr."))

# ── 2. 引数の型が異なるオーバーロード ────────────────────────────
fn type_of(x: int) -> str:
    return "int"

fn type_of(x: str) -> str:
    return "str"

fn type_of(x: float) -> str:
    return "float"

fn type_of(x: bool) -> str:
    return "bool"

print("=== 2. 引数の型が異なるオーバーロード ===")
print(type_of(42))
print(type_of("hello"))
print(type_of(3.14))
print(type_of(True))

# ── 3. 個数と型の両方が異なるオーバーロード ──────────────────────
fn add(a: int, b: int) -> int:
    return a + b

fn add(a: float, b: float) -> float:
    return a + b

fn add(a: str, b: str) -> str:
    return a + b

fn add(a: int, b: int, c: int) -> int:
    return a + b + c

print("=== 3. 個数と型の組み合わせ ===")
print(add(1, 2))
print(add(1.5, 2.5))
print(add("foo", "bar"))
print(add(1, 2, 3))

# ── 4. キーワード引数でのオーバーロード呼び出し ───────────────────
fn power(base: int, exp: int) -> int:
    mut result = 1
    for _ in range(exp):
        result *= base
    return result

fn power(base: float, exp: int) -> float:
    mut result = 1.0
    for _ in range(exp):
        result *= base
    return result

print("=== 4. キーワード引数でオーバーロード呼び出し ===")
print(power(base=2, exp=10))
print(power(base=1.5, exp=4))

# ── 5. クラスメソッドのオーバーロード ───────────────────────────
class Formatter:
    fn format(self, x: int) -> str:
        return "int"

    fn format(self, x: str) -> str:
        return "str:" + x

    fn format(self, x: int, width: int) -> str:
        return "padded"

print("=== 5. クラスメソッドのオーバーロード ===")
let fmt = Formatter()
print(fmt.format(99))
print(fmt.format("test"))
print(fmt.format(7, 10))

# ── 6. コンストラクタのオーバーロード（__init__ の多重定義） ─────
class Vector:
    mut x: int = 0
    mut y: int = 0
    mut z: int = 0

    fn __init__(mut self, x: int, y: int) -> None:
        self.x = x
        self.y = y

    fn __init__(mut self, x: int, y: int, z: int) -> None:
        self.x = x
        self.y = y
        self.z = z

print("=== 6. コンストラクタのオーバーロード ===")
let v2 = Vector(3, 4)
print("v2.x:", v2.x, " v2.y:", v2.y, " v2.z:", v2.z)
let v3 = Vector(1, 2, 5)
print("v3.x:", v3.x, " v3.y:", v3.y, " v3.z:", v3.z)

# ── 7. 継承とメソッドオーバーロードの組み合わせ ──────────────────
class Shape:
    fn area(self) -> int:
        return 0

class Rectangle(Shape):
    mut w: int = 0
    mut h: int = 0

    fn __init__(mut self, side: int) -> None:
        self.w = side
        self.h = side

    fn __init__(mut self, w: int, h: int) -> None:
        self.w = w
        self.h = h

    fn area(self) -> int:
        return self.w * self.h

print("=== 7. 継承とコンストラクタオーバーロード ===")
let square = Rectangle(5)
let rect   = Rectangle(3, 7)
print("square area:", square.area())
print("rect area:  ", rect.area())

print("=== All overload tests passed! ===")

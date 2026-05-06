# Self 型の使用例

# クラスメソッドで Self を戻り値型・パラメータ型に使用する
class Point:
    mut x: int
    mut y: int

    fn origin(self) -> Self:
        return Self(0, 0)

    fn translate(self, dx: int, dy: int) -> Self:
        return Self(self.x + dx, self.y + dy)

    fn add(self, other: Self) -> Self:
        return Self(self.x + other.x, self.y + other.y)

    fn to_str(self) -> str:
        return "Point"

let p = Point(3, 4)
let origin = p.origin()
print(origin.x)   # 0
print(origin.y)   # 0

let moved = p.translate(1, 2)
print(moved.x)    # 4
print(moved.y)    # 6

let q = Point(10, 20)
let sum = p.add(q)
print(sum.x)      # 13
print(sum.y)      # 24

# trait 内でも Self を使用できる
trait Copyable:
    fn copy(self) -> Self:
        ...

class Counter(Copyable):
    mut count: int

    fn copy(self) -> Self:
        return Self(self.count)

    fn inc(mut self) -> None:
        self.count = self.count + 1

mut c1 = Counter(5)
mut c2 = c1.copy()
c2.inc()
print(c1.count)   # 5  (元は変わらない)
print(c2.count)   # 6  (コピーした方だけ増加)

# Self をフィールド型に使用
class Node:
    mut value: int
    const ID: int = 99

    fn make_sibling(self, v: int) -> Self:
        return Self(v)

let n = Node(10)
let sibling = n.make_sibling(20)
print(sibling.value)   # 20
print(sibling.ID)      # 99

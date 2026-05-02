# class_demo.tl
# クラスの実行デモ: インスタンス化・フィールド・メソッド・継承

# ── 1. 基本クラス ────────────────────────────────────────────────
class Animal:
    mut name = ""
    fn __init__(mut self, name: str) -> None:
        self.name = name
    fn speak(self) -> str:
        return "..."
    fn describe(self) -> None:
        print(self.name, "says", self.speak())

# ── 2. 継承・メソッドオーバーライド ──────────────────────────────
class Dog(Animal):
    fn speak(self) -> str:
        return "Woof!"

class Cat(Animal):
    fn speak(self) -> str:
        return "Meow!"

print("=== 1. 継承 ===")
let d = Dog("Rex")
let c = Cat("Luna")
d.describe()
c.describe()

# ── 3. フィールドの読み書き ───────────────────────────────────────
print("=== 2. フィールド ===")
print("dog name:", d.name)

# ── 4. カウンタークラス（複合代入） ──────────────────────────────
class Counter:
    mut count = 0
    fn increment(mut self) -> None:
        self.count += 1
    fn reset(mut self) -> None:
        self.count = 0
    fn value(self) -> int:
        return self.count

print("=== 3. カウンター ===")
let counter = Counter()
counter.increment()
counter.increment()
counter.increment()
print("count:", counter.value())
counter.reset()
print("after reset:", counter.value())

# ── 5. スタッククラス ─────────────────────────────────────────────
class Stack:
    mut items = 0
    mut top = 0
    fn push(mut self, v: int) -> None:
        self.items = v
        self.top += 1
    fn size(self) -> int:
        return self.top

print("=== 4. スタック ===")
let s = Stack()
s.push(10)
s.push(20)
s.push(30)
print("stack size:", s.size())

# ── 6. キーワード引数でコンストラクタ呼び出し ────────────────────
print("=== 5. キーワード引数コンストラクタ ===")
let dog2 = Dog(name="Buddy")
print("dog2 name:", dog2.name)

print("=== All class tests passed! ===")

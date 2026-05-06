# ============================================================
# freeze 機能のサンプル
# ============================================================

# --- 1. let バインディングによる自動 freeze ---
print("=== 1. let バインディング ===")

class Point:
    mut x: int
    mut y: int
    fn dist_sq(self) -> int:
        return self.x * self.x + self.y * self.y
    fn move(mut self, dx: int, dy: int) -> None:
        self.x = self.x + dx
        self.y = self.y + dy

# let で生成したインスタンスは作成時点で全フィールドが immutable になる
let p = Point(3, 4)
print("p.x =", p.x, "  p.y =", p.y)
print("dist_sq =", p.dist_sq())    # 25

# let インスタンスなので immutable メソッドは呼べる
let d = p.dist_sq()
print("dist_sq via let =", d)

# --- 2. mut バインディング → freeze ---
print("=== 2. mut から freeze ===")

class Counter:
    mut count: int = 0
    fn inc(mut self) -> None:
        self.count = self.count + 1
    fn get(self) -> int:
        return self.count

mut c = Counter()
c.inc()
c.inc()
c.inc()
print("count before freeze:", c.get())    # 3

freeze c

# freeze 後は immutable メソッドは引き続き呼べる
print("count after  freeze:", c.get())    # 3

# --- 3. __freeze__ コールバック ---
print("=== 3. __freeze__ コールバック ===")

class Temperature:
    mut celsius: int
    mut snapshot: int = 0
    fn set(mut self, v: int) -> None:
        self.celsius = v
    fn get(self) -> int:
        return self.celsius
    fn __freeze__(mut self) -> None:
        self.snapshot = self.celsius
        print("__freeze__ called: snapshot =", self.snapshot)

mut t = Temperature(0)
t.set(37)
freeze t
print("snapshot after freeze:", t.snapshot)    # 37
print("celsius after freeze: ", t.get())       # 37

# --- 4. let インスタンスへの書き込みエラー ---
print("=== 4. エラーケース ===")

class Box:
    mut value: int = 0
    fn set(mut self, v: int) -> None:
        self.value = v
    fn get(self) -> int:
        return self.value

let b = Box()

mut ok = True

# フィールド直接書き込みはエラー
# b.value = 99    # TypeError: cannot assign to immutable field 'value'

# mut self メソッド呼び出しはエラー
# b.set(99)       # TypeError: cannot call mutable method 'set' on immutable instance

print("let Box() フィールド読み取り:", b.get())   # 0

# --- 5. freeze 後の変数再代入エラー ---
print("=== 5. freeze 後の変数再代入 ===")

class Dummy:
    mut v: int = 1

mut d2 = Dummy()
freeze d2

# d2 = Dummy()    # TypeError: cannot assign to immutable variable 'd2'

print("d2.v after freeze:", d2.v)   # 1

print("=== All freeze checks passed! ===")

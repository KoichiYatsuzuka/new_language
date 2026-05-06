# new_type の動作確認サンプル

# --- クラスから new_type を作成する ---

class Meters:
    mut value: int
    fn get(self) -> int:
        return self.value
    fn double(self) -> Self:
        return Self(self.value * 2)
    fn add(self, other: Self) -> Self:
        return Self(self.value + other.value)

new_type Kilometers: Meters

# それぞれのコンストラクタで生成
let m = Meters(100)
let km = Kilometers(5)

print(m.get())     # 100
print(km.get())    # 5

# メソッド呼び出し
let m2 = m.double()
print(m2.get())    # 200

let km2 = km.double()
print(km2.get())   # 10

# Self を使った同種インスタンス同士の演算
let m3 = m.add(m2)
print(m3.get())    # 300

let km3 = km.add(km2)
print(km3.get())   # 15

# --- プリミティブ型から new_type を作成する ---

new_type Celsius: int
new_type Fahrenheit: int

let c = Celsius(100)
let f = Fahrenheit(212)

print(c.value)     # 100
print(f.value)     # 212

# --- new_type から new_type を作成する ---

new_type LightYears: Meters

let ly = LightYears(9461)
print(ly.get())    # 9461

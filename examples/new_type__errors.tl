# new_type のエラーサンプル
# StaticTypeError: Self 型引数の型不一致

class Meters:
    mut value: int
    fn add(self, other: Self) -> Self:
        return Self(self.value + other.value)

new_type Kilometers: Meters

let m = Meters(100)
let km = Kilometers(5)

# ERROR: add の other: Self は Meters インスタンスを期待するが Kilometers を渡している
m.add(km)

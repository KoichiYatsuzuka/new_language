## trait template のテスト

# --- 事前に必要な trait ---

trait Printable:
    fn to_str(self) -> str:
        ...

# --- template パラメータ付き trait ---
# T は Printable を実装する型

trait Container[T: Printable]:
    mut item: T

# --- Printable を実装するクラス ---

class MyInt(Printable):
    mut value: int
    fn to_str(self) -> str:
        return "MyInt"

class MyStr(Printable):
    mut text: str
    fn to_str(self) -> str:
        return self.text

# --- Container[MyInt] を継承するクラス ---
# item フィールドの型変数 T が MyInt に置換される

class IntBox(Container[MyInt]):
    fn get_label(self) -> str:
        return self::Container.item.to_str()

# --- Container[MyStr] を継承するクラス ---

class StrBox(Container[MyStr]):
    fn get_label(self) -> str:
        return self::Container.item.to_str()

# --- 実行 ---

let a = MyInt(42)
let b = IntBox(a)
let s1 = b.get_label()
print(s1)

let c = MyStr("hello")
let d = StrBox(c)
let s2 = d.get_label()
print(s2)

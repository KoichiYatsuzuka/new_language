## template の動作確認

# --- trait 定義 ---

trait Printable:
    fn to_str(self) -> str:
        ...

trait Comparable:
    fn compare(self, other: int) -> int:
        ...

# --- trait を実装するクラス ---

class MyInt(Printable, Comparable):
    mut value: int
    fn to_str(self) -> str:
        return "MyInt"
    fn compare(self, other: int) -> int:
        return self.value - other

class MyStr(Printable):
    mut text: str
    fn to_str(self) -> str:
        return self.text

# --- テンプレート関数 ---

fn describe[T: Printable](item: T) -> str:
    return item.to_str()

# --- and 条件のテンプレート関数 ---

fn process[T: Printable and Comparable](item: T, threshold: int) -> str:
    let cmp = item.compare(threshold)
    if cmp > 0:
        return "greater"
    elif cmp < 0:
        return "lesser"
    else:
        return "equal"

# --- テンプレートクラス ---

class Wrapper[T: Printable]:
    mut item: T
    fn get_label(self) -> str:
        return self.item.to_str()

# --- 実行 ---

let a = MyInt(42)
let b = MyStr("hello")

# テンプレート関数の呼び出し
let s1 = describe[MyInt](a)
print(s1)

let s2 = describe[MyStr](b)
print(s2)

# and 条件のテンプレート関数
let r1 = process[MyInt](a, 10)
print(r1)

let r2 = process[MyInt](a, 42)
print(r2)

let r3 = process[MyInt](a, 100)
print(r3)

# テンプレートクラスの使用
let w = Wrapper[MyInt](a)
let label = w.get_label()
print(label)

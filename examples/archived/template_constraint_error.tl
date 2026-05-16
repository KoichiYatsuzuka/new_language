## 制約違反のテスト

trait Printable:
    fn to_str(self) -> str:
        ...

class MyInt(Printable):
    mut value: int
    fn to_str(self) -> str:
        return "MyInt"

class Plain:
    mut x: int

fn describe[T: Printable](item: T) -> str:
    return item.to_str()

let a = MyInt(99)
let ok = describe[MyInt](a)
print(ok)

# Plain は Printable トレイトを実装していない → TemplateError
let b = Plain(1)
let bad = describe[Plain](b)
print(bad)

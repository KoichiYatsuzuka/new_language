## and 条件の制約テスト

trait Printable:
    fn to_str(self) -> str:
        ...

trait Comparable:
    fn compare(self, other: int) -> int:
        ...

class Full(Printable, Comparable):
    mut value: int
    fn to_str(self) -> str:
        return "Full"
    fn compare(self, other: int) -> int:
        return self.value - other

class PrintOnly(Printable):
    mut text: str
    fn to_str(self) -> str:
        return self.text

# Printable and Comparable 両方を要求する関数
fn process[T: Printable and Comparable](item: T, threshold: int) -> str:
    let cmp = item.compare(threshold)
    if cmp >= 0:
        return "ok"
    return "low"

let f = Full(50)
let r1 = process[Full](f, 30)
print(r1)

# PrintOnly は Comparable を満たさない → TemplateError
let p = PrintOnly("hello")
let r2 = process[PrintOnly](p, 0)
print(r2)

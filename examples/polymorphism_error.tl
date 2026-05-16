# polymorphism_error.tl — TemplateError: constraint not satisfied
#
# Expected error (runtime TemplateError):
#   Instantiating a template with a type that does not implement the required trait.

trait Printable:
    fn to_str(self) -> str:
        ...

class MyInt(Printable):
    mut value: int
    fn to_str(self) -> str:
        return "MyInt"

class Plain:
    mut value: int

fn show[T: Printable](item: T) -> None:
    print(item.to_str())

let ok = MyInt(1)
show[MyInt](ok)   # OK: MyInt implements Printable

let bad = Plain(2)
show[Plain](bad)  # TemplateError: Plain does not implement Printable

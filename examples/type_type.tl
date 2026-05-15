# type[T] examples: restricting which type values are accepted

# Basic: type[int] only accepts the int type value
fn accepts_int_type(let t: type[int]) -> None:
    pass

accepts_int_type(int)   # OK: exact match

# new_type upcast: a new_type whose chain leads to int is compatible
new_type Index: int
new_type SpecialIndex: Index

accepts_int_type(Index)         # OK: new_type of int
accepts_int_type(SpecialIndex)  # OK: new_type chain -> Index -> int

# Trait upcast: a class implementing a trait is compatible with type[Trait]
trait Printable:
    pass

class Document(Printable):
    pass

fn accepts_printable(let t: type[Printable]) -> None:
    pass

accepts_printable(Document)  # OK: Document implements Printable

# bare `type` accepts any type value
fn accepts_any_type(let t: type) -> None:
    pass

accepts_any_type(int)
accepts_any_type(str)
accepts_any_type(float)
accepts_any_type(Index)
accepts_any_type(Document)

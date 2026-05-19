# class_trait_error.tl — expected class/trait/access errors
#
# Section 1 — TypeError: assigning to a let (immutable) field
#   Expected error (runtime TypeError):
#     Assigning to a let field outside of construction

class Token:
    let kind: str
    let text: str

let t = Token("ident", "foo")
print(t.kind)   # ident — OK

t.kind = "op"   # TypeError: cannot assign to immutable field 'kind'

# Section 2 — AccessError: accessing private/protected from invalid scope
#   Expected error (runtime AccessError):
#     'y' is private

class MyClass:
    public:
    mut x: int

    private:
    mut y: int

    protected:
    mut z: int

    fn __init__(mut self) -> None:
        self.x = 1
        self.y = 2
        self.z = 3

let obj = MyClass()
print(obj.y)   # AccessError: 'y' is private

# Section 3 — AttributeError: calling a static method on an instance
#   Expected error (runtime AttributeError):
#     static method 'count' must be called on the class, not an instance

class Registry:
    static mut entry_count: int = 0

    static fn count() -> int:
        return Registry.entry_count

let reg = Registry()
reg.count()   # AttributeError: static method 'count' must be called on the class, not an instance

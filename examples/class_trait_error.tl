# class_trait_error.tl — TypeError: assigning to a let (immutable) field
#
# Expected error (runtime TypeError):
#   Assigning to a let field outside of construction

class Token:
    let kind: str
    let text: str

let t = Token("ident", "foo")
print(t.kind)   # ident — OK

t.kind = "op"   # TypeError: cannot assign to immutable field 'kind'

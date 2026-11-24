use super::*;

fn class_parts(stmts: &[crate::ast::Stmt], class: &str) -> (Vec<(String, String)>, Vec<(String, Vec<String>, String)>, bool) {
    use crate::ast::Stmt;
    let Some(Stmt::ClassDef { body, decorators, .. }) =
        stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == class))
    else {
        panic!("no class {class} in {stmts:?}")
    };
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    for st in body {
        match st {
            Stmt::Field { name, type_ann, .. } => fields.push((name.clone(), type_ann.clone())),
            Stmt::FnDef { name, params, return_type, .. } => methods.push((
                name.clone(),
                params.iter().map(|p| format!("{}:{}", p.name, p.type_ann.clone().unwrap_or_default())).collect(),
                return_type.clone().unwrap_or_default(),
            )),
            _ => {}
        }
    }
    (fields, methods, !decorators.is_empty())
}

/// クラスは宣言にする: コンストラクタの引数・注釈どおりのメンバー・注釈の無いものは `Any`（7-27・利用者の決定）
#[test]
fn classes_become_declarations() {
    let src = "class Counter:\n    label: str = 'c'\n    def __init__(self, start: int, step=1) -> None:\n        self.value: int = start\n        self.step = step\n        self.a, self.b = 1, 2\n        self.value += 1\n    def bump(self) -> int:\n        return 1\n    def describe(self):\n        return ''\n    @property\n    def count(self) -> int:\n        return 0\n    @staticmethod\n    def make(n: int) -> \"Counter\":\n        return Counter(n)\n\ndef untyped(x):\n    return 1\n";
    let stmts = extract_py_type_stubs(src);
    let (fields, methods, open) = class_parts(&stmts, "Counter");
    assert_eq!(
        fields,
        vec![
            ("label".to_string(), "str".to_string()),
            ("value".to_string(), "int".to_string()),
            ("step".to_string(), "Any".to_string()),
            ("a".to_string(), "Any".to_string()),
            ("b".to_string(), "Any".to_string()),
            ("count".to_string(), "int".to_string()),
        ]
    );
    let m = |n: &str| methods.iter().find(|(name, _, _)| name == n).cloned().unwrap_or_else(|| panic!("no method {n}"));
    assert_eq!(m("__init__"), ("__init__".to_string(), vec!["self:".to_string(), "start:int".to_string(), "step:Any".to_string()], "None".to_string()));
    assert_eq!(m("bump").2, "int");
    assert_eq!(m("describe").2, "Any", "no annotation -> Any");
    assert_eq!(m("make"), ("make".to_string(), vec!["n:int".to_string()], "Counter".to_string()), "static: no self, quoted return");
    assert!(!open, "a class without bases or decorators has a closed member set");
    // 注釈の無いトップレベルの関数は Any（以前は `return 1` から int と推し量っていた）。引数も Any（7-30）
    assert!(stmts.iter().any(|s| matches!(s, crate::ast::Stmt::FnDef { name, params, return_type: Some(t), .. }
        if name == "untyped" && t == "Any" && params.len() == 1 && params[0].type_ann.as_deref() == Some("Any"))));
}

/// 基底: 同じファイルのクラスならメンバーを持つ。ほかのファイルのクラスを継承する・デコレータの付いたクラスは開いておく
#[test]
fn class_bases_and_decorators() {
    let src = "class Base:\n    def hello(self) -> str:\n        return ''\n\nclass Child(Base):\n    def own(self) -> int:\n        return 0\n\nclass Foreign(somelib.Thing):\n    pass\n\n@dataclass\nclass Data:\n    x: int\n";
    let stmts = extract_py_type_stubs(src);
    let (_, methods, open) = class_parts(&stmts, "Child");
    let names: Vec<&str> = methods.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"hello") && names.contains(&"own"), "{names:?}");
    assert!(!open);
    let (_, methods, open) = class_parts(&stmts, "Foreign");
    assert!(open, "a base outside the file: members are open");
    assert_eq!(methods[0].1, vec!["self:".to_string(), "**kwargs:dict[str, Any]".to_string(), "...:list[Any]".to_string()], "and the constructor takes anything");
    let (fields, _, open) = class_parts(&stmts, "Data");
    assert!(open, "a decorated class (dataclass adds members): open");
    assert_eq!(fields, vec![("x".to_string(), "int".to_string())]);
}

/// このファイルが宣言していないクラス・型の別名の名前は `Any`（7-30）
#[test]
fn unknown_type_names_become_any() {
    let src = "class Box:
    pass

def f(x: ArrayLike, b: Box, xs: list[Other] | None) -> Box:
    return b
";
    let stmts = extract_py_type_stubs(src);
    let Some(crate::ast::Stmt::FnDef { params, return_type, .. }) =
        stmts.iter().find(|s| matches!(s, crate::ast::Stmt::FnDef { name, .. } if name == "f"))
    else {
        panic!("no f")
    };
    let types: Vec<&str> = params.iter().map(|p| p.type_ann.as_deref().unwrap_or("")).collect();
    assert_eq!(types, vec!["Any", "Box", "Option[py_list[Any]]"]);
    assert_eq!(return_type.as_deref(), Some("Box"));
}

/// Python のバイト列の注釈はどれも `bytes`（ffi_value_exchange_plan.md 5-3）。
#[test]
fn byte_sequences_are_bytes() {
    assert_eq!(py_type_to_arrow("bytes"), "bytes");
    assert_eq!(py_type_to_arrow("bytearray"), "bytes");
    assert_eq!(py_type_to_arrow("Optional[memoryview]"), "Option[bytes]");
    assert_eq!(py_type_to_arrow("list[bytearray]"), "py_list[bytes]");
}

/// 要素型を落とさない（7-8）。以前は `dict[K, V]` → `dict`・`set[tuple[int, int]]` → `set[tuple]`・`frozenset[T]` → `Any`・
/// `Union[List[int], None]` は中を変換しないまま・`list[int] | None` → `Any` だった。
#[test]
fn element_types_are_kept() {
    assert_eq!(arrow_type("set[tuple[int, int]]"), "py_set[tuple[int, int]]");
    assert_eq!(arrow_type("frozenset[str]"), "set[str]");
    assert_eq!(arrow_type("FrozenSet[int]"), "set[int]");
    assert_eq!(arrow_type("dict[str, list[int]]"), "py_dict[str, py_list[int]]");
    assert_eq!(arrow_type("Dict[str, Tuple[int, float]]"), "py_dict[str, tuple[int, float]]");
    assert_eq!(arrow_type("Union[List[int], None]"), "Option[py_list[int]]");
    assert_eq!(arrow_type("Union[int, str, None]"), "Union[int, str, None]");
    assert_eq!(arrow_type("list[int] | None"), "Option[py_list[int]]");
    assert_eq!(arrow_type("int | str"), "Union[int, str]");
    assert_eq!(arrow_type("typing.Optional[typing.List[str]]"), "Option[py_list[str]]");
    // Arrow が表せないものは検査しない側
    assert_eq!(arrow_type("tuple[int, ...]"), "tuple");
    // `Sequence[int]` は組み込みの protocol（6-8・`typing_names_are_builtin_protocols`）。以前は `Any` だった
    assert_eq!(arrow_type("Sequence[int]"), "Sequence[int]");
    assert_eq!(arrow_type("Callable[[int], str]"), "Any");
    assert_eq!(arrow_type("Optional[Any]"), "Any");
    // 名前は読む時点では残す（このモジュールのクラスかは `py_unknown_types_to_any` が決める）
    assert_eq!(py_type_to_arrow("Person"), "Person");
    assert_eq!(arrow_type("Final[int]"), "int");
}

/// 例外のクラスは、継承をたどって行き着く組み込みの例外を基底に持つ（`except m.Err:` の型検査・pandas_import_plan.md の 2-1）。
/// 同じファイルの基底のクラスも基底に持つ（派生を基底の型で受ける・3-1）。ほかのファイルの基底は持たない（`link_py_types` が
/// 修飾名で足す）。
#[test]
fn exception_classes_keep_their_builtin_base() {
    use crate::ast::Stmt;
    let src = "class A(ValueError):\n    pass\nclass B(A):\n    pass\nclass C(builtins.KeyError):\n    pass\nclass D:\n    pass\n\
                   class E(D):\n    pass\nclass F(other.Base):\n    pass\n";
    let bases: Vec<(String, Vec<String>)> = extract_py_type_stubs(src)
        .into_iter()
        .filter_map(|s| if let Stmt::ClassDef { name, bases, .. } = s { Some((name, bases)) } else { None })
        .collect();
    let of = |n: &str| bases.iter().find(|(c, _)| c == n).map(|(_, b)| b.clone()).unwrap();
    assert_eq!(of("A"), vec!["ValueError".to_string()]);
    assert_eq!(of("B"), vec!["ValueError".to_string(), "A".to_string()]);
    assert_eq!(of("C"), vec!["KeyError".to_string()]);
    assert!(of("D").is_empty() && of("F").is_empty());
    assert_eq!(of("E"), vec!["D".to_string()]);
}

/// `@classmethod` の `cls` は呼び出しで渡さない・`@staticmethod` の先頭は本物の引数・`f = staticmethod(f)` も同じ・
/// 基底の基底のメンバーも持つ（pandas_import_plan.md 3-1）。
#[test]
fn class_and_static_methods_and_deep_bases() {
    let src = "class A:
    def __init__(self, v):
        self.v = v
    def show(self) -> str:
        return ''
class B(A):
    @classmethod
    def of(cls, v) -> 'B':
        return cls(v)
    @staticmethod
    def twice(x):
        return x
    def to_f(c):
        return c
    to_f = staticmethod(to_f)
    def parse(cls, t):
        return cls(t)
    parse = classmethod(parse)
class C(B):
    pass
";
    let stmts = extract_py_type_stubs(src);
    let (fields, methods, open) = class_parts(&stmts, "C");
    let m = |n: &str| methods.iter().find(|(name, _, _)| name == n).cloned().unwrap_or_else(|| panic!("no method {n} in {methods:?}"));
    assert_eq!(m("of").1, vec!["v:Any".to_string()], "classmethod: no cls");
    assert_eq!(m("twice").1, vec!["x:Any".to_string()], "staticmethod: the first parameter is real");
    assert_eq!(m("to_f").1, vec!["c:Any".to_string()], "`to_f = staticmethod(to_f)`");
    assert_eq!(m("parse").1, vec!["t:Any".to_string()], "`parse = classmethod(parse)`");
    assert_eq!(m("show").2, "str", "a member of the base of the base");
    assert_eq!(m("__init__").1, vec!["self:".to_string(), "v:Any".to_string()], "`__init__` of the base of the base");
    assert!(!fields.iter().any(|(n, _)| n == "to_f" || n == "parse"), "the wrappers are not attributes: {fields:?}");
    assert!(!open);
}

/// クラスのメソッドの `@overload` は、モジュールの関数と同じく本体の宣言の中に入れ子で持つ（pandas_import_plan.md の 6-4）。
/// 本体の無い形（`.pyi`）は最後の形が本体の代わりで、戻り値はすべての形を合わせたもの。形の注釈の知らない名前は `Any`（6-2）。
#[test]
fn class_method_overloads_are_nested() {
    use crate::ast::Stmt;
    let src = "class C:
    @overload
    def get(self, key: ArrayLike) -> int: ...
    @overload
    def get(self, key: str, default: str) -> str: ...
    def get(self, key, default=None):
        return key
    @overload
    def only(self, a: int) -> int: ...
    @overload
    def only(self, a: str) -> str: ...
";
    let stmts = extract_py_type_stubs(src);
    let Some(Stmt::ClassDef { body, .. }) = stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "C")) else {
        panic!("{stmts:?}")
    };
    let method = |wanted: &str| {
        let found: Vec<&Stmt> =
            body.iter().filter(|m| matches!(m, Stmt::FnDef { name, .. } if name == wanted)).collect();
        assert_eq!(found.len(), 1, "{wanted}: {body:?}");
        found[0]
    };
    let Stmt::FnDef { body: variants, return_type, .. } = method("get") else { panic!() };
    assert_eq!(variants.len(), 2);
    assert_eq!(return_type.as_deref(), Some("Any"), "the implementation has no annotation");
    assert!(variants.iter().all(is_py_overload_variant));
    let Stmt::FnDef { params, .. } = &variants[0] else { panic!() };
    assert_eq!(params[1].type_ann.as_deref(), Some("Any"), "an unknown name in a variant");
    let Stmt::FnDef { body: variants, return_type, .. } = method("only") else { panic!() };
    assert_eq!(variants.len(), 2);
    assert_eq!(return_type.as_deref(), Some("Union[int, str]"));
}

/// `*args` の後のキーワード専用引数は型を見ない。Arrow の仮引数には印が無く、位置引数が先に束縛される（6-3）。
#[test]
fn keyword_only_after_varargs_is_any() {
    let params = py_params("*shape: int, refcheck: bool = ...", false).expect("params");
    let refcheck = params.iter().find(|p| p.name == "refcheck").expect("refcheck");
    assert_eq!(refcheck.type_ann.as_deref(), Some("Any"));
    let params = py_params("a: int, *, flag: bool = ...", false).expect("params");
    let flag = params.iter().find(|p| p.name == "flag").expect("flag");
    assert_eq!(flag.type_ann.as_deref(), Some("bool"), "a bare `*` takes no positional argument anyway");
}

/// 注釈を読む 2 つの段（`py_type_to_arrow` と、知らない名前を `Any` にする `py_type_names_to_any`）を通した型。
fn arrow_type(a: &str) -> String {
    py_type_names_to_any(&py_type_to_arrow(a), &Default::default(), &Default::default(), &Default::default())
}

/// `typing` / `collections.abc` の `Sequence` などは組み込みの protocol として読む（pandas_import_plan.md の 6-8）。
/// 型引数を書かなければ `Any` で埋める。ほかのモジュールで修飾した同じ名前・個数の違う型引数は検査しない（`Any`）。
#[test]
fn typing_names_are_builtin_protocols() {
    assert_eq!(arrow_type("Sequence[int]"), "Sequence[int]");
    assert_eq!(arrow_type("typing.Iterable[str]"), "Iterable[str]");
    assert_eq!(arrow_type("Mapping[str, int]"), "Mapping[str, int]");
    assert_eq!(arrow_type("Sequence"), "Sequence[Any]");
    assert_eq!(arrow_type("collections.abc.Mapping"), "Mapping[Any, Any]");
    assert_eq!(arrow_type("SupportsIndex"), "SupportsIndex");
    assert_eq!(arrow_type("Iterable[int] | None"), "Option[Iterable[int]]");
    assert_eq!(arrow_type("Mapping[str]"), "Any", "wrong number of type arguments");
    assert_eq!(arrow_type("Sized[int]"), "Any", "Sized takes no type arguments");
    assert_eq!(arrow_type("np.Sized"), "Any", "another module's Sized is not the built-in protocol");
}

/// 小文字のクラス（`ndarray`）と総称のクラスの注釈（`ndarray[int]` は `ndarray`）を型にする。境界で Arrow の数になるクラスは
/// 境界と同じ型で読む: 長さを持たず `__index__` を持つ物は `int`、`__float__` などだけを持つ物は `Any`、長さを持つ物はクラス（6-11）。
#[test]
fn lowercase_and_numeric_classes() {
    use crate::ast::Stmt;
    let src = "class ndarray:
    def __len__(self) -> int: ...
    def __index__(self) -> int: ...
class integer:
    def __index__(self) -> int: ...
    def __getitem__(self, key) -> integer: ...
class floating:
    def __float__(self) -> float: ...
class dtype:
    pass
def f(a: ndarray[int], b: integer, c: floating, d: dtype[floating], e: unknown_cls) -> ndarray: ...
";
    let stmts = extract_py_type_stubs(src);
    let Some(Stmt::FnDef { params, return_type, .. }) =
        stmts.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "f"))
    else {
        panic!("no f");
    };
    let ann: Vec<&str> = params.iter().filter_map(|p| p.type_ann.as_deref()).collect();
    assert_eq!(ann, ["ndarray", "int", "Any", "dtype", "Any"]);
    assert_eq!(return_type.as_deref(), Some("ndarray"));
}

/// 型の別名（`X = 型の式` / `X: TypeAlias = ..` / `type X[T] = ..`・複数行）を集め、注釈で展開する（6-10）。型変数は中身に現れた順の
/// 型引数。値の代入（`MAX = 10` / `f = g()` / `__all__ = [..]`）は別名ではない。
#[test]
fn type_aliases_are_expanded() {
    use crate::ast::Stmt;
    let src = "from typing import TypeAlias, TypeVar, Union
_T = TypeVar('_T')
IntList = list[int]
Pair = tuple[_T, _T]
Number: TypeAlias = Union[int, float]
Nested = Union[
    int,
    IntList,
]
type Grid[T] = list[list[T]]
MAX = 10
helper = make()
__all__ = ['f']
def f(a: IntList, b: Pair[str], c: Number, d: Nested, e: Grid[int], g: Pair) -> None: ...
";
    let aliases = py_type_aliases(src);
    assert_eq!(aliases.get("Pair").map(|a| a.params.clone()), Some(vec!["_T".to_string()]));
    assert!(!aliases.contains_key("MAX") && !aliases.contains_key("helper") && !aliases.contains_key("__all__"));
    let stmts = extract_py_type_stubs(src);
    let Some(Stmt::FnDef { params, .. }) = stmts.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "f")) else {
        panic!("no f");
    };
    let ann: Vec<&str> = params.iter().filter_map(|p| p.type_ann.as_deref()).collect();
    assert_eq!(
        ann,
        [
            "py_list[int]",
            "tuple[str, str]",
            "Union[int, float]",
            "Union[int, py_list[int]]",
            "py_list[py_list[int]]",
            "tuple[Any, Any]",
        ]
    );
}

/// `class X(Protocol[_T_co])` は Arrow の protocol（型変数は型引数・`__init__` は要求に入れない）。本体の注釈の型変数と、
/// このモジュールの `_` で始まる名前は残す。知らない名前は型引数ごと `Any`、このモジュールのクラスの型引数は落とす（6-9）。
#[test]
fn protocol_classes_become_protocols() {
    use crate::ast::Stmt;
    let src = "from typing import Protocol, TypeVar
_T_co = TypeVar('_T_co')
class SupportsRead(Protocol[_T_co]):
    def read(self, n: int) -> _T_co: ...
class _Closer(Protocol):
    def close(self) -> None: ...
class Box:
    pass
def f(r: SupportsRead[str], c: _Closer, b: Box[int], u: Unknown[int], t: _T_co) -> None: ...
";
    let stmts = extract_py_type_stubs(src);
    let Some(Stmt::ProtocolDef { template_params, body, .. }) =
        stmts.iter().find(|s| matches!(s, Stmt::ProtocolDef { name, .. } if name == "SupportsRead"))
    else {
        panic!("SupportsRead is not a protocol: {stmts:?}");
    };
    assert_eq!(template_params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["_T_co"]);
    let Some(Stmt::FnDef { return_type, .. }) = body.iter().find(|m| matches!(m, Stmt::FnDef { name, .. } if name == "read"))
    else {
        panic!("no read");
    };
    assert_eq!(return_type.as_deref(), Some("_T_co"));
    assert!(!body.iter().any(|m| matches!(m, Stmt::FnDef { name, .. } if name == "__init__")));
    assert!(stmts.iter().any(|s| matches!(s, Stmt::ProtocolDef { name, .. } if name == "_Closer")));
    let Some(Stmt::FnDef { params, .. }) = stmts.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "f")) else {
        panic!("no f");
    };
    let ann: Vec<&str> = params.iter().filter_map(|p| p.type_ann.as_deref()).collect();
    // ⚠ 型変数は protocol の本体の外では型の名前ではない（`_T_co` は `Any`）
    assert_eq!(ann, ["SupportsRead[str]", "_Closer", "Box", "Any", "Any"]);
}

/// 入れ子のクラスは `Outer.Inner` の名前で持ち上げ、外側のクラスにはその型の値の属性を置く。入れ子のクラスの基底は同じ本体の
/// 兄弟を先に引く。本体に宣言でない文（`if`）があるクラスは開いておく（pandas_import_plan.md 3-1）。
#[test]
fn nested_classes_are_hoisted() {
    use crate::ast::Stmt;
    let src = "class Node:
    pass
class Tree:
    class Node:
        def __init__(self, value):
            self.value = value
    class Leaf(Node):
        pass
class Settings:
    debug = False
    if debug:
        level = 10
";
    let stmts = extract_py_type_stubs(src);
    let names: Vec<&str> = stmts.iter().filter_map(|s| if let Stmt::ClassDef { name, .. } = s { Some(name.as_str()) } else { None }).collect();
    assert_eq!(names, vec!["Node", "Tree.Node", "Tree.Leaf", "Tree", "Settings"]);
    let (fields, _, _) = class_parts(&stmts, "Tree");
    assert_eq!(fields, vec![("Node".to_string(), "type[Tree.Node]".to_string()), ("Leaf".to_string(), "type[Tree.Leaf]".to_string())]);
    let (_, methods, _) = class_parts(&stmts, "Tree.Leaf");
    assert_eq!(methods[0].1, vec!["self:".to_string(), "value:Any".to_string()], "the sibling `Node`, not the top-level one");
    let Some(Stmt::ClassDef { bases, .. }) = stmts.iter().find(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Tree.Leaf")) else {
        panic!()
    };
    assert_eq!(bases, &vec!["Tree.Node".to_string()]);
    let (_, _, open) = class_parts(&stmts, "Settings");
    assert!(open, "a class body with an `if`: the members are open");
}

/// 注釈のあるモジュールの大域変数は型つきの `let` の宣言になる（pandas_import_plan.md の 1-4）。注釈の無いもの・`_` で
/// 始まるもの・1 行の複合文（`else: x = 1`）は拾わない。
#[test]
fn annotated_module_globals_are_declared() {
    use crate::ast::Stmt;
    let src = "version: str\nmaxsize: int = 9\nlimit: Final[float] = 1.0\nplain = 3\n_hidden: int = 1\n\
                   if x:\n    pass\nelse: y = 1\nclass C:\n    n: int\n";
    let globals: Vec<(String, Option<String>)> = extract_py_type_stubs(src)
        .into_iter()
        .filter_map(|s| if let Stmt::Let(n, t, _, _) = s { Some((n, t)) } else { None })
        .collect();
    assert_eq!(
        globals,
        vec![
            ("version".to_string(), Some("str".to_string())),
            ("maxsize".to_string(), Some("int".to_string())),
            ("limit".to_string(), Some("float".to_string())),
        ]
    );
}

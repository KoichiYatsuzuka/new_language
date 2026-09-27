// tests/classes.rs — クラス定義・継承・メソッド、およびトレイトのテスト。

use super::*;

// --- classes ---

/// class_instantiate のテスト。
#[test]
fn test_class_instantiate() {
    // Fields have defaults → no required args → Point() is the right call.
    let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\nlet p = Point()\n";
    assert!(run(src).is_ok());
}

/// class_instantiate_required_fields のテスト。
#[test]
fn test_class_instantiate_required_fields() {
    // Fields without defaults → auto-init requires args.
    let src = "class Point:\n    mut x: int\n    mut y: int\nlet p = Point(3, 4)\n";
    assert!(run(src).is_ok());
}

/// class_init_sets_field のテスト。
#[test]
fn test_class_init_sets_field() {
    let src = "class Dog:\n    mut name: str = \"\"\n    fn __init__(mut self, name: str) -> None:\n        self.name = name\nlet d = Dog(\"Rex\")\n";
    assert!(run(src).is_ok());
}

/// class_method_call のテスト。
#[test]
fn test_class_method_call() {
    let src = "class Greeter:\n    fn greet(self) -> str:\n        return \"hello\"\nlet g = Greeter()\nlet r = g.greet()\n";
    if let Value::Str(s) = run_get(src, "r") {
        assert_eq!(&*s, "hello");
    } else {
        panic!();
    }
}

/// class_field_access のテスト。
#[test]
fn test_class_field_access() {
    // Fields have defaults; use defaults when instantiating.
    let src =
        "class Pair:\n    mut x: int = 10\n    mut y: int = 20\nlet p = Pair()\nlet r = p.x\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

/// class_field_access_required のテスト。
#[test]
fn test_class_field_access_required() {
    // Fields without defaults require constructor args.
    let src = "class Pair:\n    mut x: int\n    mut y: int\nlet p = Pair(10, 20)\nlet r = p.x\n";
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 10);
    } else {
        panic!();
    }
}

/// access_public_field_ok のテスト。
#[test]
fn test_access_public_field_ok() {
    let src = concat!(
        "class C:\n",
        "    public:\n",
        "    mut x: int = 42\n",
        "let obj = C()\n",
        "let r = obj.x\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

/// access_private_field_from_outside_errors のテスト。
#[test]
fn test_access_private_field_from_outside_errors() {
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut secret: int = 99\n",
        "let obj = C()\n",
        "let r = obj.secret\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
    assert!(msg.contains("private"), "expected 'private', got: {msg}");
}

/// access_private_field_from_method_ok のテスト。
#[test]
fn test_access_private_field_from_method_ok() {
    // ⚠ `public:` が無いと `get_secret` も private になる（以前はメソッドだけ見ていなかった・10-6）。
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut secret: int = 99\n",
        "    public:\n",
        "    fn get_secret(self) -> int:\n",
        "        return self.secret\n",
        "let obj = C()\n",
        "let r = obj.get_secret()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 99);
    } else {
        panic!();
    }
}

/// access_protected_field_from_outside_errors のテスト。
#[test]
fn test_access_protected_field_from_outside_errors() {
    let src = concat!(
        "trait Guarding:\n",
        "    protected:\n",
        "    mut guarded: int\n",
        "class C(Guarding):\n",
        "    pass\n",
        "let obj = C(7)\n",
        "let r = obj.guarded\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
    assert!(
        msg.contains("protected"),
        "expected 'protected', got: {msg}"
    );
}

/// access_protected_field_via_method_ok のテスト。
#[test]
fn test_access_protected_field_via_method_ok() {
    let src = concat!(
        "trait Guarding:\n",
        "    protected:\n",
        "    mut guarded: int\n",
        "class C(Guarding):\n",
        "    fn get_guarded(self) -> int:\n",
        "        return self.guarded\n",
        "let obj = C(7)\n",
        "let r = obj.get_guarded()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 7);
    } else {
        panic!();
    }
}

/// access_mixed_sections_in_class のテスト。
#[test]
fn test_access_mixed_sections_in_class() {
    let src = concat!(
        "class C:\n",
        "    public:\n",
        "    mut visible: int = 1\n",
        "    private:\n",
        "    mut hidden: int = 2\n",
        // ⚠ `public:` が無いと `get_hidden` も private になる（以前はメソッドだけ見ていなかった・10-6）。
        "    public:\n",
        "    fn get_hidden(self) -> int:\n",
        "        return self.hidden\n",
        "let obj = C()\n",
        "let pub_val = obj.visible\n",
        "let priv_val = obj.get_hidden()\n",
    );
    if let (Value::Int(a), Value::Int(b)) = (run_get(src, "pub_val"), run_get(src, "priv_val")) {
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    } else {
        panic!();
    }
}

/// access_private_write_from_outside_errors のテスト。
#[test]
fn test_access_private_write_from_outside_errors() {
    let src = concat!(
        "class C:\n",
        "    private:\n",
        "    mut x: int = 0\n",
        "mut obj = C()\n",
        "obj.x = 5\n",
    );
    let result = run(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("AccessError"),
        "expected AccessError, got: {msg}"
    );
}

/// class_self_field_in_method のテスト。
#[test]
fn test_class_self_field_in_method() {
    let src = concat!(
        "class Box:\n",
        "    mut value: int = 0\n",
        "    fn set(mut self, v: int) -> None:\n",
        "        self.value = v\n",
        "    fn get(self) -> int:\n",
        "        return self.value\n",
        "mut b = Box()\n", // mut: instance will be mutated via set()
        "b.set(42)\n",
        "let r = b.get()\n",
    );
    if let Value::Int(n) = run_get(src, "r") {
        assert_eq!(n, 42);
    } else {
        panic!();
    }
}

/// class_inheritance_non_trait_parse_error のテスト。
#[test]
fn test_class_inheritance_non_trait_parse_error() {
    // Class-to-class inheritance is not supported; must use traits instead.
    let src = concat!(
        "class Animal:\n",
        "    fn speak(self) -> str:\n",
        "        return \"...\"\n",
        "class Dog(Animal):\n",
        "    fn speak(self) -> str:\n",
        "        return \"Woof\"\n",
    );
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "expected parse error for class-to-class inheritance"
    );
    assert!(result.unwrap_err().contains("cannot inherit from `Animal`"));
}

/// class_inherit_non_trait_base_parse_error のテスト。
#[test]
fn test_class_inherit_non_trait_base_parse_error() {
    // Class-to-class inheritance is no longer supported; the parser must reject it.
    let src = concat!(
        "class Base:\n",
        "    fn hello(self) -> str:\n",
        "        return \"hi\"\n",
        "class Child(Base):\n",
        "    pass\n",
    );
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let result = crate::parser::Parser::new(tokens, None).parse_program();
    assert!(
        result.is_err(),
        "expected parse error for class-to-class inheritance"
    );
    assert!(result.unwrap_err().contains("cannot inherit from `Base`"));
}

// --- trait ---

/// trait_class_instantiate_combined_constructor のテスト。
#[test]
fn test_trait_class_instantiate_combined_constructor() {
    // Class inheriting a trait; combined __init__ takes trait fields then class fields.
    let src = concat!(
        "trait HasValue:\n",
        "    mut value: int\n",
        "class Container(HasValue):\n",
        "    mut tag: str\n",
        "let c = Container(42, \"hello\")\n",
    );
    assert!(run(src).is_ok());
}

/// trait_field_read_via_class_method のテスト。
#[test]
fn test_trait_field_read_via_class_method() {
    // A method defined in the CLASS body reads a trait field via TraitAccess.
    let src = concat!(
        "trait HasValue:\n",
        "    mut value: int\n",
        "class Container(HasValue):\n",
        "    mut tag: str\n",
        "    fn get_value(self) -> int:\n",
        "        return self::HasValue.value\n",
        "    fn get_tag(self) -> str:\n",
        "        return self.tag\n",
        "let c = Container(99, \"hi\")\n",
        "let v = c.get_value()\n",
        "let t = c.get_tag()\n",
    );
    if let Value::Int(n) = run_get(src, "v") {
        assert_eq!(n, 99);
    } else {
        panic!("expected int for v");
    }
    if let Value::Str(s) = run_get(src, "t") {
        assert_eq!(&*s, "hi");
    } else {
        panic!("expected str for t");
    }
}

/// trait_virtual_override_executes のテスト。
#[test]
fn test_trait_virtual_override_executes() {
    // Virtual method overridden in class; override body actually runs.
    let src = concat!(
        "trait Shape:\n",
        "    fn area(self) -> float:\n",
        "        ...\n",
        "class Square(Shape):\n",
        "    mut side: float\n",
        "    fn area(self) -> float:\n",
        "        return self.side * self.side\n",
        "let s = Square(3.0)\n",
        "let a = s.area()\n",
    );
    if let Value::Float(f) = run_get(src, "a") {
        assert!((f - 9.0).abs() < 1e-9, "expected 9.0, got {f}");
    } else {
        panic!("expected float for a");
    }
}

/// trait_only_required_fields_no_class_fields のテスト。
#[test]
fn test_trait_only_required_fields_no_class_fields() {
    // Class body has no required fields; only the trait's required field.
    let src = concat!(
        "trait Named:\n",
        "    mut name: str\n",
        "class Widget(Named):\n",
        "    fn get_name(self) -> str:\n",
        "        return self::Named.name\n",
        "let w = Widget(\"button\")\n",
        "let n = w.get_name()\n",
    );
    if let Value::Str(s) = run_get(src, "n") {
        assert_eq!(&*s, "button");
    } else {
        panic!("expected str for n");
    }
}


// ── private / protected メソッド（フェーズ10 10-6）──────────────────────────
// ⚠ 以前はメソッド呼び出しだけアクセス指定を見ておらず、クラスの外から private メソッドを
//   呼べた。ここは**実行時の**検査を見る（`run` は型エラーを無視する・静的な検査は
//   `type_check_tests/access.rs`）。

const ACCOUNT_RT: &str = concat!(
    "class Account:\n",
    "    mut balance: int\n",
    "    fn deposit(mut self, let n: int) -> None:\n",
    "        self.audit(n)\n",
    "        self.balance += n\n",
    "    static fn open() -> Account:\n",
    "        return Account(Account.start(), 0)\n",
    "    static fn peek(let a: Account) -> int:\n",
    "        return a.audit(0) + a.secret\n",
    "    private:\n",
    "    mut secret: int\n",
    "    fn audit(self, let n: int) -> int:\n",
    "        return n\n",
    "    static fn start() -> int:\n",
    "        return 7\n",
    "    gen items(self) -> int:\n",
    "        yield self.balance\n",
);

/// 最初の誤りを「クラス名: 中身」で返す。関数の中で出た誤りは例外として伝播する
/// （`run_err_msg` には `__raise__` しか返らない）ので、例外の中身を取り出す。
fn first_error(src: &str) -> String {
    match run_exc(src) {
        Err(e) => e,
        Ok(None) => String::new(),
        Ok(Some(raised)) => match &raised.exception {
            Value::Instance(inst) => {
                let b = inst.borrow();
                let fields: Vec<String> = b
                    .class
                    .field_index
                    .values()
                    .filter_map(|&i| b.field_value(i))
                    .map(|v| format!("{v:?}"))
                    .collect();
                format!("{}: {}", b.class.name, fields.join(" "))
            }
            other => format!("{other:?}"),
        },
    }
}

fn account_err(tail: &str) -> String {
    first_error(&format!("{ACCOUNT_RT}{tail}"))
}

/// private メソッドをクラスの外から呼ぶと `AccessError`。
#[test]
fn test_private_method_from_outside_errors() {
    let msg = account_err("mut a = Account(0, 0)\nlet r = a.audit(5)\n");
    assert!(msg.contains("AccessError") && msg.contains("'audit' is private"), "{msg}");
}

/// クラスの中（インスタンスメソッド・静的メソッド）からは呼べる。
#[test]
fn test_private_method_from_inside_ok() {
    let src = format!("{ACCOUNT_RT}mut a = Account.open()\na.deposit(5)\nlet r = a.balance\n");
    assert_int(run_get(&src, "r"), 12);
}

/// 静的メソッドの中はクラスの中（private のフィールド・メソッドに届く）。
/// ⚠ 以前は静的メソッドの中が「外」扱いで、private のフィールドも `AccessError` だった。
#[test]
fn test_static_method_reaches_private_members() {
    let src = format!("{ACCOUNT_RT}let r = Account.peek(Account(1, 0))\n");
    assert_int(run_get(&src, "r"), 0);
}

/// private の静的メソッドをクラス経由で外から呼ぶと `AccessError`。
#[test]
fn test_private_static_method_from_outside_errors() {
    let msg = account_err("let r = Account.start()\n");
    assert!(msg.contains("'start' is private"), "{msg}");
}

/// private の gen メソッドも同じ。
#[test]
fn test_private_gen_method_from_outside_errors() {
    let msg = account_err("let a = Account(3, 0)\nlet g = a.items()\n");
    assert!(msg.contains("'items' is private"), "{msg}");
}

/// 別のクラスのメソッドから呼ぶのも「外」。
#[test]
fn test_private_method_from_other_class_errors() {
    let msg = account_err(concat!(
        "class Auditor:\n",
        "    mut n: int\n",
        "    fn check(self, let a: Account) -> int:\n",
        "        return a.audit(1)\n",
        "let r = Auditor(0).check(Account(0, 0))\n",
    ));
    assert!(msg.contains("'audit' is private"), "{msg}");
}

/// method IC に焼いたアクセスレベルも見る。同じ呼び出し位置（`peek` の中の `a.audit(0)`）を
/// クラスの中から通して IC を埋めたあと、同じ関数をクラスの文脈なしで呼ぶと止まる。
/// ⚠ IC 命中の経路が検査を飛ばすと、2 回目が通ってしまう。
#[test]
fn test_private_method_ic_hit_still_checks() {
    let msg = account_err(concat!(
        "let ok = Account.peek(Account(1, 0))\n",
        "let f = Account.peek\n",
        "let r = f(Account(1, 0))\n",
    ));
    assert!(msg.contains("'audit' is private"), "{msg}");
}

/// メソッドの中の入れ子の関数は、定義したときのクラスの文脈で走る。
/// ⚠ 以前は入れ子の関数を呼ぶと文脈が消え、private のフィールドも `AccessError` だった。
#[test]
fn test_nested_fn_in_method_reaches_private_members() {
    let src = concat!(
        "class C:\n",
        "    mut n: int\n",
        "    fn f(self) -> int:\n",
        "        fn g() -> int:\n",
        "            return self.s + self.h()\n",
        "        return g()\n",
        "    private:\n",
        "    mut s: int\n",
        "    fn h(self) -> int:\n",
        "        return 10\n",
        "let r = C(0, 1).f()\n",
    );
    assert_int(run_get(src, "r"), 11);
}

/// 入れ子の関数を外へ返しても文脈は定義したときのクラス（字句の規則）。
#[test]
fn test_escaped_nested_fn_keeps_class_context() {
    let src = concat!(
        "class C:\n",
        "    mut n: int\n",
        "    fn getter(self) -> function[]->int:\n",
        "        fn g() -> int:\n",
        "            return self.h()\n",
        "        return g\n",
        "    private:\n",
        "    fn h(self) -> int:\n",
        "        return 10\n",
        "let g = C(0).getter()\n",
        "let r = g()\n",
    );
    assert_int(run_get(src, "r"), 10);
}

/// メソッドの中の入れ子の `gen` も同じ。
#[test]
fn test_nested_gen_in_method_reaches_private_members() {
    let src = concat!(
        "class C:\n",
        "    mut n: int\n",
        "    fn total(self) -> int:\n",
        "        gen g() -> int:\n",
        "            yield self.h()\n",
        "            yield self.s\n",
        "        mut t = 0\n",
        "        for x in g():\n",
        "            t += x\n",
        "        return t\n",
        "    private:\n",
        "    mut s: int\n",
        "    fn h(self) -> int:\n",
        "        return 10\n",
        "let r = C(0, 1).total()\n",
    );
    assert_int(run_get(src, "r"), 11);
}

/// trait の protected メソッドを外から呼ぶと `AccessError`。
#[test]
fn test_protected_method_from_outside_errors() {
    let msg = first_error(concat!(
        "trait Guarding:\n",
        "    protected:\n",
        "    fn guard(self) -> int:\n",
        "        return 1\n",
        "class C(Guarding):\n",
        "    mut n: int\n",
        "    fn use_guard(self) -> int:\n",
        "        return self.guard()\n",
        "let c = C(0)\n",
        "let ok = c.use_guard()\n",
        "let r = c.guard()\n",
    ));
    assert!(msg.contains("'guard' is protected"), "{msg}");
}

/// 外へ返した入れ子の関数を、別の関数の中（VM の呼び出し経路）から呼んでも文脈は保たれる。
#[test]
fn test_escaped_nested_fn_called_from_vm_keeps_class_context() {
    let src = concat!(
        "class C:\n",
        "    mut n: int\n",
        "    fn getter(self) -> function[]->int:\n",
        "        fn g() -> int:\n",
        "            return self.h()\n",
        "        return g\n",
        "    private:\n",
        "    fn h(self) -> int:\n",
        "        return 10\n",
        "fn call_it(let f: function[]->int) -> int:\n",
        "    return f() + 1\n",
        "let r = call_it(C(0).getter())\n",
    );
    assert_int(run_get(src, "r"), 11);
}

/// 具体化を値として使う（フェーズ10 10-9）: 静的メソッド・`static mut`（具体化ごとに別）・クラスそのもの。
/// ⚠ 以前は `Stack[int].empty()` が添字として読まれ `'template' object is not subscriptable`、
///   `Counter[T].n` は `NameError: 'T'` だった。
#[test]
fn test_template_instance_as_value() {
    let src = concat!(
        "class Stack[T]:\n",
        "    mut items: list[T]\n",
        "    static fn empty() -> Stack[T]:\n",
        "        return Stack[T]([])\n",
        "class Counter[T]:\n",
        "    static mut n: int = 0\n",
        "    mut v: T\n",
        "    fn bump(self) -> None:\n",
        "        Counter[T].n += 1\n",
        "let s = Stack[int].empty()\n",
        "let size = len(s.items)\n",
        "let a = Counter[int](1)\n",
        "a.bump()\n",
        "a.bump()\n",
        "Counter[str](\"x\").bump()\n",
        "let ni = Counter[int].n\n",
        "let ns = Counter[str].n\n",
        "let mk = Stack[str]\n",
        "let t = mk([\"a\"])\n",
        "let tsize = len(t.items)\n",
    );
    assert_int(run_get(src, "size"), 0);
    assert_int(run_get(src, "ni"), 2);
    assert_int(run_get(src, "ns"), 1);
    assert_int(run_get(src, "tsize"), 1);
}

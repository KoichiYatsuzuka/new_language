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

// ---------------------------------------------------------------------------
// 束縛メソッド（python_builtins_plan.md のタスク 1-8）
// ---------------------------------------------------------------------------

/// `obj.method` を値として読むと受け手を束縛した関数になり、`obj.method(..)` と同じ実体が走る。
/// コールバック（`map` / `sorted(key=)`）に渡せ、同じ受け手の同じメソッドは等しい。
/// 不変の受け手の `mut self` のメソッドは、束縛はできるが呼ぶと誤り（直接呼ぶのと同じ判定）。
#[test]
fn test_bound_methods() {
    let src = concat!(
        "class Counter:\n",
        "    mut n: int\n",
        "    fn add(mut self, let k: int) -> int:\n",
        "        self.n = self.n + k\n",
        "        return self.n\n",
        "    fn get(self) -> int:\n",
        "        return self.n\n",
        "mut c = Counter(1)\n",
        "let add = c.add\n",
        "let r1 = add(10)\n",
        "let r2 = c.get()\n",
        "let mapped = str(list(map(c.add, [1, 2])))\n",
        "let same = c.get == c.get\n",
        "let differ = c.get == c.add\n",
        "let shown = str(c.get)\n",
    );
    assert!(matches!(run_get(src, "r1"), Value::Int(11)));
    assert!(matches!(run_get(src, "r2"), Value::Int(11)));
    assert_str(run_get(src, "mapped"), "[12, 14]");
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
    assert!(matches!(run_get(src, "differ"), Value::Bool(false)));
    let shown = run_get(src, "shown");
    let Value::Str(s) = shown else { panic!("not a str") };
    assert!(s.starts_with("<bound method Counter.get of <Counter object at 0x"), "{s}");
    let immutable = concat!(
        "class Counter:\n",
        "    mut n: int\n",
        "    fn add(mut self, let k: int) -> int:\n",
        "        self.n = self.n + k\n",
        "        return self.n\n",
        "let c = Counter(1)\n",
        "let add = c.add\n",
        "let r = add(1)\n",
    );
    assert!(run_err_msg(immutable).contains("cannot call mutable method 'add' on immutable instance of 'Counter'"));
}

/// `gen` メソッドと組み込みの値のメソッドも束縛できる（タスク 1-10）。組み込みの値のメソッドは同じ値を書き換え、
/// 同じ受け手の同じ名前は等しい。
#[test]
fn test_bound_gen_and_builtin_methods() {
    let src = concat!(
        "class Bag:\n",
        "    let xs: list[int]\n",
        "    gen each(self) -> int:\n",
        "        for x in self.xs:\n",
        "            yield x * 10\n",
        "let bag = Bag([1, 2])\n",
        "let each = bag.each\n",
        "let gens = str(list(each()))\n",
        "mut out: list[int] = []\n",
        "let push = out.append\n",
        "push(7)\n",
        "let pushed = str(out)\n",
        "let up = \"ab\".upper\n",
        "let upper = up()\n",
        "let same = out.append == out.append\n",
        "let shown = str(out.append)\n",
    );
    assert_str(run_get(src, "gens"), "[10, 20]");
    assert_str(run_get(src, "pushed"), "[7]");
    assert_str(run_get(src, "upper"), "AB");
    assert!(matches!(run_get(src, "same"), Value::Bool(true)));
    let Value::Str(s) = run_get(src, "shown") else { panic!("not a str") };
    assert!(s.starts_with("<built-in method append of list object at 0x"), "{s}");
}

/// 組み込みの値のメソッドの表（`names::BUILTIN_VALUE_METHODS`）の名前は、実行時のメソッドの入口が
/// **知っている**（「無いメソッド」の誤りにならない）。表だけに足して実行時に無い名前を作らないための検査。
/// ⚠ 逆向き（実行時にあるのに表に無い）は見ない。足し忘れは値として読めないだけで、黙って壊れはしない。
#[test]
fn builtin_value_methods_are_dispatched() {
    let src = concat!(
        "class V:\n",
        "    mut x: float\n",
        "mut st = \"a b\"\n",
        "mut li = [1, 2]\n",
        "mut di = {\"a\": 1}\n",
        "mut se = {1, 2}\n",
        "mut fl: fixed_list[V] = [V(1.0)] => fixed_list[V]\n",
        "mut ge = [1, 2].__iter__()\n",
        "mut fz = frozenset([1, 2])\n",
        "mut by = bytes([97, 98])\n",
        "mut fo = open(\"target/test_tmp_value_methods.txt\", \"w\")\n",
    );
    let (stmts, mut interp) = prepare(src).unwrap();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    for (kind, names) in crate::type_check::names::BUILTIN_VALUE_METHODS {
        let var = match *kind {
            "str" => "st",
            "list" => "li",
            "dict" => "di",
            "set" => "se",
            "fixed_list" => "fl",
            "generator" => "ge",
            "frozenset" => "fz",
            "bytes" => "by",
            "FileObject" => "fo",
            other => panic!("no sample receiver for '{other}'"),
        };
        let recv = interp.get_val(var).unwrap();
        let unknown = interp
            .eval_method_call_full(recv.clone(), "__no_such_method__", Vec::new(), None)
            .expect_err("an unknown method must fail");
        for name in names.iter() {
            if let Err(e) = interp.eval_method_call_full(recv.clone(), name, Vec::new(), None) {
                assert_ne!(
                    e,
                    unknown.replace("__no_such_method__", name),
                    "'{kind}.{name}' is in BUILTIN_VALUE_METHODS but the runtime does not know it"
                );
            }
        }
    }
}

/// メソッドの無い名前の `obj.f(x)` は属性を読んでその値を呼ぶ（python_builtins_plan.md のタスク 1-11）。
/// 以前は `has no method 'f'` だった。
#[test]
fn test_call_a_callable_stored_in_a_field() {
    let src = concat!(
        "fn double(let x: int) -> int:\n",
        "    return x * 2\n",
        "class Holder:\n",
        "    let f: function[let int]->int\n",
        "    fn run(self, let x: int) -> int:\n",
        "        return self.f(x)\n",
        "let h = Holder(double)\n",
        "let a = h.run(21)\n",
        "let b = h.f(4)\n",
    );
    assert!(matches!(run_get(src, "a"), Value::Int(42)));
    assert!(matches!(run_get(src, "b"), Value::Int(8)));
}

/// ディスクリプタのプロトコル（python_builtins_plan.md のタスク 1-12）の共通部分。
/// `Twice` はデータディスクリプタ（`static mut`）、`Label` は非データディスクリプタ（`const`）。
const DESCRIPTOR_CLASSES: &str = concat!(
    "class Twice:\n",
    "    fn __get__(self, mut obj: Any, owner: Any = None) -> Any:\n",
    "        if obj is None:\n",
    "            return \"class\"\n",
    "        return obj.n * 2\n",
    "    fn __set__(self, mut obj: Any, value: Any) -> None:\n",
    "        obj.n = value // 2\n",
    "class Label:\n",
    "    fn __get__(self, mut obj: Any, owner: Any = None) -> Any:\n",
    "        return \"n=\" + str(obj.n)\n",
    "class Box:\n",
    "    mut n: int\n",
    "    static mut twice: Twice = Twice()\n",
    "    const label: Label = Label()\n",
    "    fn __init__(mut self, n: int) -> None:\n",
    "        self.n = n\n",
);

/// クラス側のディスクリプタは、インスタンスから読むと `__get__(obj, C)`、代入すると `__set__(obj, v)`、
/// クラスから読むと `__get__(None, C)`（タスク 1-12）。以前は値そのもの（`<Twice object>`）が返り、
/// 代入は `static mut` を書き換えていた。
#[test]
fn test_descriptor_get_set_and_class_access() {
    let src = format!(
        "{DESCRIPTOR_CLASSES}{}",
        concat!(
            "mut b = Box(5)\n",
            "let r1 = b.twice\n",
            "b.twice = 40\n",
            "let n = b.n\n",
            "let r2 = b.twice\n",
            "let lab = b.label\n",
            "let via_class = Box.twice\n",
            "let other = Box(1).twice\n",
        )
    );
    let interp = run_interp(&src);
    assert!(matches!(interp.get_val("r1"), Some(Value::Int(10))));
    assert!(matches!(interp.get_val("n"), Some(Value::Int(20))));
    assert!(matches!(interp.get_val("r2"), Some(Value::Int(40))));
    assert!(matches!(interp.get_val("lab"), Some(Value::Str(s)) if s.to_string() == "n=20"));
    assert!(matches!(interp.get_val("via_class"), Some(Value::Str(s)) if s.to_string() == "class"));
    // `static mut` のディスクリプタは書き換わっていない（代入は `__set__` へ行った）。
    assert!(matches!(interp.get_val("other"), Some(Value::Int(2))));
}

/// `__delete__` だけを持つデータディスクリプタへの代入は CPython と同じ `AttributeError: __set__`。
#[test]
fn test_descriptor_without_set_rejects_assignment() {
    let src = concat!(
        "class NoSet:\n",
        "    fn __get__(self, obj: Any, owner: Any = None) -> Any:\n",
        "        return 1\n",
        "    fn __delete__(self, obj: Any) -> None:\n",
        "        pass\n",
        "class C:\n",
        "    static mut d: NoSet = NoSet()\n",
        "mut c = C()\n",
        "c.d = 3\n",
    );
    assert!(run_err_msg(src).contains("AttributeError: __set__"), "{}", run_err_msg(src));
}

/// `__py_delattr`（Python の `del o.a`）はフィールドを未代入に戻す。raw レイアウト（int / float だけのクラス・init の
/// ビットマップ）と boxed レイアウトのどちらも、消した後は `hasattr` が偽で、また代入できる（known_bugs.md の #2）。
#[test]
fn test_py_delattr_resets_field_in_both_layouts() {
    let src = concat!(
        "class Raw:\n",
        "    mut x: int\n",
        "    mut y: float\n",
        "class Boxed:\n",
        "    mut s: str\n",
        "    mut n: int\n",
        "mut r = Raw(1, 2.0)\n",
        "mut b = Boxed(\"a\", 3)\n",
        "__py_delattr(r, \"x\")\n",
        "__py_delattr(b, \"s\")\n",
        "let gone = not hasattr(r, \"x\") and hasattr(r, \"y\") and not hasattr(b, \"s\") and hasattr(b, \"n\")\n",
        "r.x = 5\n",
        "b.s = \"z\"\n",
        "let back = r.x * 10 + b.n\n",
        "let s_back = b.s\n",
    );
    let interp = run_interp(src);
    let layouts = ["r", "b"].map(|v| match interp.get_val(v) {
        Some(Value::Instance(i)) => i.borrow().has_raw_layout(),
        other => panic!("{v}: {other:?}"),
    });
    assert_eq!(layouts, [true, false], "the test needs one class of each layout");
    assert!(matches!(interp.get_val("gone"), Some(Value::Bool(true))));
    assert!(matches!(interp.get_val("back"), Some(Value::Int(53))));
    assert!(matches!(interp.get_val("s_back"), Some(Value::Str(s)) if s.to_string() == "z"));
    // 消した欄をもう一度消すのは CPython と同じ `AttributeError`、不変のインスタンスは `TypeError`。
    let twice = concat!("class P:\n", "    mut x: int\n", "mut p = P(1)\n", "__py_delattr(p, \"x\")\n", "__py_delattr(p, \"x\")\n");
    assert!(run_err_msg(twice).contains("AttributeError: 'P' object has no attribute 'x'"), "{}", run_err_msg(twice));
    let frozen = concat!("class P:\n", "    mut x: int\n", "let p = P(1)\n", "__py_delattr(p, \"x\")\n");
    assert!(
        run_err_msg(frozen).contains("cannot delete field 'x' on immutable instance"),
        "{}",
        run_err_msg(frozen)
    );
}


/// `str` / `print` は `__str__` が無ければ `__repr__`、コンテナの要素は `repr`（タスク 1-15・CPython と同じ）。
#[test]
fn test_str_falls_back_to_repr_and_containers_use_repr() {
    let src = concat!(
        "class T:\n",
        "    mut v: int\n",
        "    fn __init__(mut self, let v: int) -> None:\n",
        "        self.v = v\n",
        "    fn __repr__(self) -> str:\n",
        "        return \"T(\" + str(self.v) + \")\"\n",
        "class S:\n",
        "    fn __str__(self) -> str:\n",
        "        return \"s\"\n",
        "    fn __repr__(self) -> str:\n",
        "        return \"S()\"\n",
        "let t = T(1)\n",
        "let a = str(t)\n",
        "let b = str([t, S()])\n",
        "let c = f\"{(t,)}\"\n",
        "let d = str({\"k\": t})\n",
        "let e = str(S())\n",
    );
    let interp = run_interp(src);
    let s = |name: &str| match interp.get_val(name) {
        Some(Value::Str(s)) => s.to_string(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(s("a"), "T(1)");
    assert_eq!(s("b"), "[T(1), S()]");
    assert_eq!(s("c"), "(T(1),)");
    assert_eq!(s("d"), "{'k': T(1)}");
    assert_eq!(s("e"), "s");
}

/// `object()` のインスタンスは写さずに共有し、同一性で比べる（python_builtins_plan.md のタスク 6-10）。
/// ⚠ 等値（`values_eq`）とハッシュ（`default_hash`）が同じ規則であること（ずれると「入れたのに引けない辞書」）と、
///   格納で写す関数（`deep_copy_value` / `deep_copy_unfrozen`）が同じ物を返すことを固定する。
#[test]
fn object_instances_are_shared_and_compared_by_identity() {
    use crate::interpreter::{DictData, Interpreter, Value};
    let mut interp = Interpreter::new();
    let a = interp.py_object(&[]).expect("object()");
    let b = interp.py_object(&[]).expect("object()");
    let same_rc = |x: &Value, y: &Value| matches!((x, y), (Value::Instance(p), Value::Instance(q)) if std::rc::Rc::ptr_eq(p, q));
    // 写さない（フィールド・辞書・リストへの格納と `copy()` が通る関数）
    assert!(same_rc(&Interpreter::deep_copy_value(a.clone()), &a));
    assert!(same_rc(&Interpreter::deep_copy_unfrozen(a.clone()), &a));
    // 同一性で比べる・ハッシュも同じ規則
    assert!(Interpreter::values_eq_pure(&a, &a.clone()).unwrap());
    assert!(!Interpreter::values_eq_pure(&a, &b).unwrap());
    assert_eq!(Interpreter::default_hash(&a).unwrap(), Interpreter::default_hash(&a.clone()).unwrap());
    // 別々の object() は辞書のキーとしても別
    let mut d = DictData::new("Any".to_string(), "Any".to_string());
    Interpreter::dict_set_pure(&mut d, a.clone(), Value::Int(1)).unwrap();
    Interpreter::dict_set_pure(&mut d, b.clone(), Value::Int(2)).unwrap();
    Interpreter::dict_set_pure(&mut d, a.clone(), Value::Int(3)).unwrap();
    assert_eq!(d.len(), 2);
    assert!(matches!(Interpreter::dict_get_pure(&d, &a), Ok(Some(Value::Int(3)))));
    // すべての値は object の派生（型の判定だけ）
    for v in [Value::Int(1), Value::None, Value::str("s"), a.clone()] {
        assert_eq!(crate::interpreter::ops::typecheck::primitive_ann_matches("object", &v), Some(true));
    }
}

/// `Ellipsis` は唯一の値（python_builtins_plan.md のタスク 6-11）。スレッドへ送る複製（`deep_clone`・別の `Rc`）も
/// 同じ物として扱う（`is` / `==` / `hash` が種類で決まる）。格納では写さない。
#[test]
fn ellipsis_is_one_value_even_across_thread_copies() {
    use crate::interpreter::{BuiltinClass, Interpreter, Value};
    let mut interp = Interpreter::new();
    let e = interp.builtin_singleton(BuiltinClass::Ellipsis);
    let again = interp.builtin_singleton(BuiltinClass::Ellipsis);
    let sent = e.deep_clone();
    let same_rc = |x: &Value, y: &Value| matches!((x, y), (Value::Instance(p), Value::Instance(q)) if std::rc::Rc::ptr_eq(p, q));
    assert!(same_rc(&e, &again));
    assert!(same_rc(&Interpreter::deep_copy_value(e.clone()), &e));
    assert!(!same_rc(&sent, &e));
    assert!(interp.values_ref_eq(&e, &sent).unwrap());
    assert!(Interpreter::values_eq_pure(&e, &sent).unwrap());
    assert_eq!(Interpreter::default_hash(&e).unwrap(), Interpreter::default_hash(&sent).unwrap());
    assert!(!Interpreter::values_eq_pure(&e, &Value::None).unwrap());
    assert_eq!(interp.display(&e), "Ellipsis");
}

/// 格納の写し（`copy_for_store`）は Arrow のコードの中だけ。Python から変換したコードの中は写さずに共有する
/// （known_bugs.md の #1・A 案）。どちらのコードかは `running_python`（チャンクを走らせる入口が差し替える）。
#[test]
fn copy_for_store_shares_inside_python_code() {
    use crate::interpreter::{Interpreter, Value};
    let mut interp = Interpreter::new();
    let xs = Value::List(std::rc::Rc::new(std::cell::RefCell::new(vec![Value::Int(1)])));
    let same = |a: &Value, b: &Value| matches!((a, b), (Value::List(p), Value::List(q)) if std::rc::Rc::ptr_eq(p, q));
    assert!(!interp.running_python);
    assert!(!same(&interp.copy_for_store(xs.clone()), &xs), "Arrow code copies on store");
    interp.running_python = true;
    assert!(same(&interp.copy_for_store(xs.clone()), &xs), "Python code shares on store");
}

// type_check_tests/variables.rs — 変数の再宣言・不変代入・不変フィールド代入の静的型検査テスト。

use super::*;

    // --- Variable redeclaration ---

    /// let_redeclaration_same_scope のテスト。
    #[test]
    fn let_redeclaration_same_scope() {
        assert!(err("let a = 5\nlet a = 6\n"));
    }

    /// mut_redeclaration_same_scope のテスト。
    #[test]
    fn mut_redeclaration_same_scope() {
        assert!(err("mut a = 5\nmut a = 6\n"));
    }

    /// let_then_mut_redeclaration のテスト。
    #[test]
    fn let_then_mut_redeclaration() {
        assert!(err("let a = 5\nmut a = 6\n"));
    }

    /// const_redeclaration_same_scope のテスト。
    #[test]
    fn const_redeclaration_same_scope() {
        assert!(err("const A = 5\nconst A = 6\n"));
    }

    /// redeclaration_in_inner_scope のテスト（外側スコープの変数と同名）。
    #[test]
    fn redeclaration_in_inner_scope() {
        assert!(err("let x = 1\nif True:\n    let x = 2\n"));
    }

    /// redeclaration_in_function_body のテスト（外側の let と同名）。
    #[test]
    fn redeclaration_in_function_body() {
        assert!(err("let x = 1\nfn f() -> None:\n    let x = 2\n"));
    }

    /// redeclaration_tuple_target のテスト。
    #[test]
    fn redeclaration_tuple_target() {
        assert!(err("let a = 1\nlet a, let b = (2, 3)\n"));
    }

    /// underscore_redeclaration_allowed のテスト（_ は再宣言を許可）。
    #[test]
    fn underscore_redeclaration_allowed() {
        assert!(ok("let _ = 1\nlet _ = 2\n"));
    }

    /// redeclaration_error_mentions_name のテスト（エラーメッセージに変数名が含まれる）。
    #[test]
    fn redeclaration_error_mentions_name() {
        let errors = check("let foo = 1\nlet foo = 2\n");
        assert!(!errors.is_empty());
        let msg = errors[0].to_string();
        assert!(msg.contains("foo"), "error should mention variable name, got: {msg}");
        assert!(msg.contains("already declared"), "error should say 'already declared', got: {msg}");
    }

    // --- Immutable assignment ---

    /// let_immutable_assign のテスト。
    #[test]
    fn let_immutable_assign() {
        assert!(err("let x = 1\nx = 2"));
    }

    /// const_immutable_assign のテスト。
    #[test]
    fn const_immutable_assign() {
        assert!(err("const X = 1\nX = 2"));
    }

    /// mut_assign_ok のテスト。
    #[test]
    fn mut_assign_ok() {
        assert!(ok("mut x = 1\nx = 2"));
    }

    /// let_compound_assign_immutable のテスト。
    #[test]
    fn let_compound_assign_immutable() {
        assert!(err("let x = 1\nx += 1"));
    }

    /// mut_compound_assign_ok のテスト。
    #[test]
    fn mut_compound_assign_ok() {
        assert!(ok("mut x = 1\nx += 1"));
    }

    /// immutable_assign_inside_if のテスト。
    #[test]
    fn immutable_assign_inside_if() {
        assert!(err("let x = 1\nif True:\n    x = 2\n"));
    }

    /// mut_assign_inside_if_ok のテスト。
    #[test]
    fn mut_assign_inside_if_ok() {
        assert!(ok("mut x = 1\nif True:\n    x = 2\n"));
    }

    // --- Immutable field assignment ---

    /// let_field_assign_outside_class_err のテスト。
    #[test]
    fn let_field_assign_outside_class_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "let t = Token(\"ident\")\n",
            "t.kind = \"op\"\n",
        )));
    }

    /// let_field_assign_in_other_method_err のテスト。
    #[test]
    fn let_field_assign_in_other_method_err() {
        assert!(err(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn reset(mut self) -> None:\n",
            "        self.kind = \"op\"\n",
        )));
    }

    /// let_field_assign_in_init_ok のテスト。
    #[test]
    fn let_field_assign_in_init_ok() {
        assert!(ok(concat!(
            "class Token:\n",
            "    let kind: str\n",
            "    fn __init__(mut self, k: str) -> None:\n",
            "        self.kind = k\n",
        )));
    }

    /// `mut` 束縛のフィールドへの代入は通る。
    #[test]
    fn mut_binding_field_assign_ok() {
        assert!(ok(concat!(
            "class Counter:\n",
            "    mut count: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.count = 0\n",
            "mut c = Counter()\n",
            "c.count = 5\n",
        )));
    }

    /// `let` 束縛のフィールドへの代入は**静的エラー**（タスク 7.2・検体 `M5`）。
    ///
    /// ⚠⚠ **このテストは以前 `mut_field_assign_ok` という名前で `ok(..)` を主張していた。**
    /// フィールドが `mut count: int` なら通る、という読みだったが、**実行時は同じコードを
    /// 拒否する**（`TypeError: cannot assign to immutable field 'count'`）。
    /// ⇒ テストが静的検査の穴を仕様として固定していた形。実行時に合わせて `err` に直した。
    ///
    /// 見るべきは「**束縛**が `let` か」で、「フィールドが `let` 宣言か」は別の検査
    /// （`check_immutable_field_assign`）。
    #[test]
    fn let_binding_field_assign_err() {
        assert!(err(concat!(
            "class Counter:\n",
            "    mut count: int\n",
            "    fn __init__(mut self) -> None:\n",
            "        self.count = 0\n",
            "let c = Counter()\n",
            "c.count = 5\n",
        )));
    }

    /// let_field_compound_assign_outside_err のテスト。
    #[test]
    fn let_field_compound_assign_outside_err() {
        assert!(err(concat!(
            "class Node:\n",
            "    let value: int\n",
            "let n = Node(1)\n",
            "n.value += 1\n",
        )));
    }

    /// **`const` への代入は、どの経路でも同じ `AssignToConst`**（`enum_member_type_plan.md` 5-2）。
    ///
    /// ⚠ 以前はインスタンス経由（`c.LIMIT`）だけが弾かれ（`AssignToImmutableField`）、クラス名経由
    ///   （`Counter.LIMIT`）と `__init__` の中の `self.LIMIT` は実行時の `TypeError` まで通っていた。
    /// ⚠ enum のメンバー（暗黙に `const`）は `tests/enum_defaults.rs` で見ている。
    #[test]
    fn const_member_assign_err_on_every_path() {
        const COUNTER: &str = concat!(
            "class Counter:\n",
            "    const LIMIT: int = 100\n",
            "    static mut total: int = 0\n",
            "    mut own: int\n",
        );
        let is_limit = |errs: &[StaticTypeError]| {
            errs.iter().any(|e| matches!(&e.kind,
                TypeErrorKind::AssignToConst { owner, member } if owner == "Counter" && member == "LIMIT"))
        };
        for (body, what) in [
            ("Counter.LIMIT = 5\n", "class name"),
            ("Counter.LIMIT += 1\n", "compound"),
            ("mut c = Counter(1)\nc.LIMIT = 5\n", "instance"),
        ] {
            let errs = check(&format!("{COUNTER}{body}"));
            assert!(is_limit(&errs), "{what}: expected AssignToConst, got {errs:?}");
        }
        // `__init__` の中でも代入できない（`let` フィールドの初回代入とは別）。
        let errs = check(concat!(
            "class Counter:\n",
            "    const LIMIT: int = 100\n",
            "    mut own: int\n",
            "    fn __init__(mut self, n: int) -> None:\n",
            "        self.own = n\n",
            "        self.LIMIT = 7\n",
        ));
        assert!(is_limit(&errs), "__init__: expected AssignToConst, got {errs:?}");
        // 負の対照: `static mut` はクラス名経由・インスタンス経由とも代入できる。
        assert!(ok(&format!("{COUNTER}Counter.total = 5\nmut c = Counter(1)\nc.total = 9\n")));
    }

    /// **`const` の中身も書き換えられない**（経路のどこに `const` があっても・`ModifyConst`・5-3）。
    ///
    /// ⚠ 以前は根の束縛（`mut c`）だけを見ていたので、`c.L[0] = 2` / `c.L.append(2)` / `c.O.x = 2` /
    ///   メソッドの中の `self.L.append(2)` が通り、クラスで共有する `const` を書き換えていた。
    #[test]
    fn const_contents_are_read_only_on_every_path() {
        const SRC: &str = concat!(
            "class P:\n",
            "    mut x: int\n",
            "trait T:\n",
            "    const TL: list[int] = [1]\n",
            "class C(T):\n",
            "    const L: list[int] = [1]\n",
            "    const O: P = P(1)\n",
            "    mut n: int\n",
        );
        let is_modify = |errs: &[StaticTypeError], member: &str| {
            errs.iter().any(|e| matches!(&e.kind,
                TypeErrorKind::ModifyConst { owner, member: m } if owner == "C" && m == member))
        };
        for (body, member) in [
            ("C.L[0] = 2\n", "L"),
            ("C.L.append(2)\n", "L"),
            ("C.O.x = 2\n", "O"),
            ("mut c = C(1)\nc.L[0] = 2\n", "L"),
            ("mut c = C(1)\nc.L.append(2)\n", "L"),
            ("mut c = C(1)\nc.O.x = 2\n", "O"),
            ("mut c = C(1)\nc.TL[0] = 2\n", "TL"),
        ] {
            let errs = check(&format!("{SRC}{body}"));
            assert!(is_modify(&errs, member), "{body:?}: expected ModifyConst, got {errs:?}");
        }
        // メソッドの中の `self.L` も同じ（`mut self` でも）。
        let errs = check(&format!("{SRC}    fn bump(mut self) -> None:\n        self.L.append(2)\n"));
        assert!(is_modify(&errs, "L"), "self.L.append: expected ModifyConst, got {errs:?}");
        // trait の `const` への代入はクラスの `const` と同じ `AssignToConst`。
        let errs = check(&format!("{SRC}mut c = C(1)\nc.TL = [2]\n"));
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::AssignToConst { member, .. } if member == "TL")),
            "trait const: expected AssignToConst, got {errs:?}"
        );
        // `mut` の仮引数へは渡せない（書き戻しで中身が変わる）。
        assert!(err(&format!("{SRC}fn f(mut xs: list[int]) -> None:\n    xs.append(1)\nf(C.L)\n")));
        // 負の対照: 読む・写しを書き換える・`let` の仮引数へ渡すのは通る。
        assert!(ok(&format!(
            "{SRC}fn g(let xs: list[int]) -> int:\n    return xs[0]\nprint(g(C.L))\nmut copy = C.L\ncopy.append(3)\nprint(C.L[0], C.O.x)\n"
        )));
    }

    /// **`static mut` の中身はクラス名経由でも書ける**（5-3 で直した誤検出）。
    ///
    /// ⚠ 以前は経路の根のクラス名（`Counter`）の束縛が不変なので、`Counter.items.append(1)` が
    ///   `cannot call 'append' on 'Counter' — it is immutable` で弾かれていた（実行時は通る）。
    ///   型の値・モジュールは「値の束縛」ではない（代入の検査と同じ区別）。
    #[test]
    fn static_mut_contents_writable_through_class_name() {
        assert!(ok(concat!(
            "class Counter:\n",
            "    static mut items: list[int] = []\n",
            "    mut own: int\n",
            "Counter.items.append(1)\n",
            "Counter.items[0] = 2\n",
        )));
    }

// ── メタプログラミングの型（設計書 §1.2 / §1.5・タスク 1-5）────────────────

/// ⚠⚠ `Code` は**メタ関数の外へ持ち出せない**（設計書 §1.2）。
/// 展開時にしか存在しない値なので、通常の変数に束縛できると
/// 「実行時に評価できない値」が型検査をすり抜けて実行まで届く。
///
/// ⚠ `code:` そのものはメタ関数の外では**パースで**弾かれる（タスク 1-6）。ここで見るのは
/// 残る経路 —— **純粋メタ関数の返り値**（`Code`）を持ち出す形。展開しないエディタでは
/// この検査が唯一の網になる。
fn escapes(source: &str) -> bool {
    check(source)
        .iter()
        .any(|e| matches!(e.kind, TypeErrorKind::CodeEscapesMetafunction { .. }))
}

#[test]
fn code_value_cannot_be_bound_outside_a_metafunction() {
    assert!(escapes(concat!(
        "exprconst fn frag() -> Code:\n",
        "    let a = code:\n",
        "        print(1)\n",
        "    return a\n",
        "let f = frag()\n",
    )));
}

/// ⚠ 普通の関数の中でも同じ。`Code` を扱えるのはメタ関数の中だけ。
#[test]
fn code_value_cannot_escape_through_a_plain_function() {
    assert!(escapes(concat!(
        "exprconst fn frag() -> Code:\n",
        "    let a = code:\n",
        "        pass\n",
        "    return a\n",
        "fn g() -> None:\n",
        "    let b = frag()\n",
    )));
}

/// メタ情報型は**注釈として書ける**（設計書 §1.5）。
/// ⚠ Arrow は関数の仮引数の注釈が必須なので、これが無いとメタ関数そのものが書けない。
#[test]
fn meta_type_names_are_accepted_as_annotations() {
    use crate::type_check::InferredType;
    for name in ["Code", "meta_instance", "meta_function", "meta_class", "meta_member"] {
        assert!(
            InferredType::from_ann(name).is_some(),
            "`{name}` を型注釈として解釈できること"
        );
    }
}

/// ⚠ `meta_*` は**別々の型**。フィールドとメソッドを混ぜない（設計書 §1.5）。
#[test]
fn meta_kinds_are_distinct_types() {
    use crate::type_check::InferredType;
    let member = InferredType::from_ann("meta_member").unwrap();
    let function = InferredType::from_ann("meta_function").unwrap();
    assert_ne!(member, function);
    assert_eq!(member.to_string(), "meta_member");
    assert_eq!(function.to_string(), "meta_function");
}

    // --- 変更できない受け手への `mut self` メソッド（フェーズ10 10-15）---
    // ⚠ 以前は実行時のガード任せで、`let` / `freeze` は実行時の TypeError、`let` 仮引数・
    //   `mut` でない `self` の先は写しを黙って書き換えて何も起きなかった。

    const BOX: &str = concat!(
        "class Box:\n",
        "    mut v: int\n",
        "    fn set(mut self, let x: int) -> None:\n",
        "        self.v = x\n",
        "    fn peek(self) -> int:\n",
        "        return self.v\n",
    );

    /// `freeze` した束縛へ `mut self` メソッド（10-15 の再現）。
    #[test]
    fn mut_self_method_on_frozen_err() {
        assert!(err(&format!("{BOX}mut b = Box(1)\nfreeze b\nb.set(2)\n")));
    }

    /// `let` 束縛へ `mut self` メソッド。
    #[test]
    fn mut_self_method_on_let_err() {
        assert!(err(&format!("{BOX}let b = Box(1)\nb.set(2)\n")));
    }

    /// `let` 仮引数へ `mut self` メソッド（実行時は写しを黙って書き換えていた）。
    #[test]
    fn mut_self_method_on_let_param_err() {
        assert!(err(&format!("{BOX}fn f(let p: Box) -> None:\n    p.set(3)\n")));
    }

    /// `mut` でない `self` の先のフィールドへ `mut self` メソッド。
    #[test]
    fn mut_self_method_through_immutable_self_err() {
        assert!(err(&format!(
            "{BOX}class Holder:\n    mut b: Box\n    fn poke(self) -> None:\n        self.b.set(5)\n"
        )));
    }

    /// `mut` 束縛・`mut` 仮引数・`mut self` の先は通る。`mut self` でないメソッドは `let` でも通る。
    #[test]
    fn mut_self_method_on_mutable_ok() {
        assert!(ok(&format!(concat!(
            "{}",
            "mut b = Box(1)\n",
            "b.set(2)\n",
            "fn f(mut p: Box) -> None:\n",
            "    p.set(3)\n",
            "f(b)\n",
            "let c = Box(4)\n",
            "print(c.peek())\n",
            "class Holder:\n",
            "    mut b: Box\n",
            "    fn poke(mut self) -> None:\n",
            "        self.b.set(5)\n",
        ), BOX)));
    }

    /// 一時値（根が識別子でない）は通す。
    #[test]
    fn mut_self_method_on_temporary_ok() {
        assert!(ok(&format!("{BOX}Box(1).set(2)\n")));
    }

    // --- 未定義の名前（フェーズ10 10-12）---
    // ⚠ 以前は静的に通り、実行時の NameError だった。判定は保守的（`type_check::names` の doc）。

    fn undefined_names(src: &str) -> Vec<String> {
        check(src)
            .iter()
            .filter_map(|e| match &e.kind {
                TypeErrorKind::UndefinedName { name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// どこにも無い名前（10-12 の再現）。
    #[test]
    fn undefined_name_err() {
        assert_eq!(undefined_names("print(nonexistent(2))\n"), vec!["nonexistent".to_string()]);
    }

    /// 後で宣言される最上位の名前を関数の中から読むのは通る（前方参照）。
    #[test]
    fn forward_reference_ok() {
        assert!(undefined_names("fn f() -> int:\n    return LIMIT + g()\nfn g() -> int:\n    return 1\nlet LIMIT = 3\nprint(f())\n").is_empty());
    }

    /// 組み込みの名前・`case _:`・`for` の変数・`except` の別名・クラス・列挙は通る。
    #[test]
    fn builtin_and_bound_names_ok() {
        let src = concat!(
            "class P:\n",
            "    mut x: int\n",
            "enum Color:\n",
            "    Red\n",
            "let r = Ok(1)\n",
            "let e = Err(\"bad\")\n",
            "for i in range(2):\n",
            "    print(i, len([1]), repr(i))\n",
            "try:\n",
            "    raise ValueError(\"v\")\n",
            "except ValueError as err:\n",
            "    print(err.message)\n",
            "match 1:\n",
            "    case 1:\n",
            "        print(P(1).x, Color.Red)\n",
            "    case _:\n",
            "        print(\"other\")\n",
        );
        assert!(undefined_names(src).is_empty(), "{:?}", undefined_names(src));
    }


    /// 式の側に位置が無い誤りは**文の位置**で報告する（フェーズ10 10-17。以前は `<unknown>`）。
    #[test]
    fn error_without_expression_position_uses_statement_position() {
        // 右辺が識別子（位置を持たない）の宣言。
        let errs = check("let s = 1\nlet z: str = s\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
        let at = errs[0].span.as_ref().map(|s| (s.line, s.col));
        assert_eq!(at, Some((2, 1)), "{errs:?}");
        // 式文の中の呼び出しの引数の誤り（式文は文の位置を持たないので、呼び出しの位置で代える）。
        let errs = check("fn f(let x: int) -> None:\n    pass\n\nf(\"s\")\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].span.as_ref().map(|s| s.line), Some(4), "{errs:?}");
    }

    /// 組み込みの型名はどのスレッドでも大域で見える名前で、同じ名前の変数は宣言できない（7-25・利用者の決定）。
    /// ⚠ 以前は型検査の大域に無い名前（`Signal` / `Size` / `Index` / `pointer` / `EventLoop`）だけ型検査を通り、実行時に
    ///   `NameError` になった。ファイルのパスの型は `Path`（以前は `path`）なので `path` は普通の名前。組み込みの関数は隠せる。
    #[test]
    fn builtin_type_names_cannot_be_redeclared() {
        for name in ["Signal", "Size", "Index", "pointer", "EventLoop", "Path", "AsyncManager", "int"] {
            assert!(err(&format!("let {name} = 1\n")), "{name}");
        }
        assert!(err("fn f() -> None:\n    let Signal = 1\n"));
        assert!(err("for Index in [1]:\n    pass\n"));
        let src = "let path = \"a.txt\"\nlet p = Path(path)\n";
        assert!(ok(src), "{:?}", check(src));
        assert!(ok("let repr = 1\n"));
    }

    /// 外部言語との境界の例外（ffi_value_exchange_plan.md 7-4）は組み込みの例外クラス。`except` に書け、
    /// `FfiTypeError` は `TypeError` の欄にも入る（親は例外の表 `names::BUILTIN_EXCEPTIONS`・実行時も同じ表）。
    #[test]
    fn ffi_exceptions_are_builtin_exception_classes() {
        let src = concat!(
            "try:
",
            "    raise ForeignError(\"boom\")
",
            "except ForeignError as e:
",
            "    print(e.message)
",
            "except FfiTypeError as e:
",
            "    print(e.message)
",
            "let t: TypeError = FfiTypeError(\"f\")
",
            "let x: Exception = ForeignError(\"g\")
",
        );
        assert!(ok(src), "{:?}", check(src));
    }

    /// `Exception` はすべての例外の基底（フェーズ10 10-19）。組み込みの例外も `Error` を実装した利用者の
    /// 例外も `Exception` の欄・引数に入る。`except Error` は `except Exception` を促す誤り。
    #[test]
    fn exception_is_the_base_of_every_exception() {
        let src = concat!(
            "class MyErr(Error):\n",
            "    fn __init__(mut self, message: str) -> None:\n",
            "        self.message = message\n",
            "fn take(let e: Exception) -> str:\n",
            "    return e.message\n",
            "let a: Exception = ValueError(\"v\")\n",
            "let b = take(MyErr(\"m\"))\n",
            "let c = take(KeyError(\"k\"))\n",
        );
        assert!(ok(src), "{:?}", check(src));
        // 例外でない値は入らない。
        assert!(err("let d: Exception = 5\n"));
        let errs = check("try:\n    pass\nexcept Error as e:\n    pass\n");
        assert!(
            errs.iter().any(|e| matches!(e.kind, TypeErrorKind::ExceptOnErrorTrait)),
            "{errs:?}"
        );
    }

    /// 組み込みの例外は CPython の階層を持つ（python_builtins_plan.md のタスク 4-1）。子は親の欄に入り、
    /// 親は子の欄に入らない。足した例外・警告のクラスも `except` に書ける。
    #[test]
    fn builtin_exceptions_follow_the_cpython_hierarchy() {
        let src = concat!(
            "let a: LookupError = KeyError(\"k\")\n",
            "let b: OSError = FileNotFoundError(\"f\")\n",
            "let c: IOError = PermissionError(\"p\")\n",
            "let d: BaseException = ValueError(\"v\")\n",
            "let w: Warning = FutureWarning(\"w\")\n",
            "try:\n    pass\nexcept UserWarning as e:\n    pass\n",
        );
        assert!(ok(src), "{:?}", check(src));
        assert!(err("let e: KeyError = LookupError(\"x\")\n"));
        assert!(err("let e: Exception = GeneratorExit(\"g\")\n"));
    }

    /// 束縛メソッドには型が付く（タスク 1-11）: `self` を除いたシグネチャの関数型で、引数・戻り値を検査する。
    /// 不変の受け手を書き換えるメソッドは、束縛はできるが呼ぶと `MutatingMethodOnImmutable`（直接呼ぶのと同じ）。
    /// 組み込みの値のメソッドは表にある名前だけ読める。
    #[test]
    fn bound_methods_are_typed() {
        let class = concat!(
            "class Counter:\n",
            "    mut n: int\n",
            "    fn add(mut self, let k: int) -> int:\n",
            "        self.n = self.n + k\n",
            "        return self.n\n",
            "    fn get(self) -> int:\n",
            "        return self.n\n",
        );
        let ok_src = format!(
            "{class}mut c = Counter(1)\nlet g = c.get\nlet x: int = g()\nlet a = c.add\nlet y = a(2)\nlet fixed = Counter(2)\nlet fa = fixed.add\nmut xs: list[int] = []\nlet push = xs.append\npush(1)\nlet up = \"s\".upper\n"
        );
        assert!(ok(&ok_src), "{:?}", check(&ok_src));
        assert!(err(&format!("{class}mut c = Counter(1)\nlet g = c.get\nlet s: str = g()\n")));
        assert!(err(&format!("{class}mut c = Counter(1)\nlet a = c.add\nlet y = a(\"x\")\n")));
        let errs = check(&format!("{class}let fixed = Counter(2)\nlet fa = fixed.add\nlet r = fa(1)\n"));
        assert!(
            errs.iter().any(|e| matches!(e.kind, TypeErrorKind::MutatingMethodOnImmutable { .. })),
            "{errs:?}"
        );
        let errs = check("let ys: list[int] = [1]\nlet p = ys.append\np(2)\n");
        assert!(
            errs.iter().any(|e| matches!(e.kind, TypeErrorKind::MutatingMethodOnImmutable { .. })),
            "{errs:?}"
        );
        let errs = check("mut zs: list[int] = [1]\nlet f = zs.no_such\n");
        assert!(errs.iter().any(|e| matches!(e.kind, TypeErrorKind::NoSuchMember { .. })), "{errs:?}");
    }

    /// 関数を入れたフィールドを `obj.f(x)` で呼ぶ形（タスク 1-11）はフィールドの関数型で検査する。
    /// 呼べない値を入れたフィールドは値を呼ぶのと同じ誤り。
    #[test]
    fn calling_a_function_field_is_typed() {
        let class = concat!(
            "fn double(let x: int) -> int:\n",
            "    return x * 2\n",
            "class Holder:\n",
            "    let f: function[let int]->int\n",
            "let h = Holder(double)\n",
        );
        assert!(ok(&format!("{class}let n: int = h.f(4)\n")));
        assert!(err(&format!("{class}let s: str = h.f(4)\n")));
        assert!(err(&format!("{class}let n = h.f(\"x\")\n")));
        assert!(err("class Box:\n    let n: int\nlet b = Box(3)\nlet v = b.n(1)\n"));
    }

    /// frozenset の型（タスク 3-1）: `frozenset(xs)` は `frozenset[要素]`、演算子の結果は左の種類、
    /// `set[T]` とは互いに代入できない。set との `==`・包含の比較（`<=`）は通る。素の `frozenset` は注釈に書けない。
    #[test]
    fn frozenset_is_its_own_type() {
        assert!(ok("let a: frozenset[int] = frozenset([1, 2])\n"));
        assert!(ok("let e: frozenset[int] = frozenset()\n"));
        assert!(err("let a: frozenset[str] = frozenset([1, 2])\n"));
        assert!(err("let s: set[int] = frozenset([1])\n"));
        assert!(err("let f: frozenset[int] = {1}\n"));
        let fs = "let a = frozenset([1, 2])\nlet s: set[int] = {2}\n";
        assert!(ok(&format!("{fs}let u: frozenset[int] = a | s\nlet v: set[int] = s | a\n")));
        assert!(ok(&format!("{fs}let b: bool = a == s\nlet c: bool = a <= s\nlet d: bool = 1 in a\n")));
        assert!(ok(&format!("{fs}let i: frozenset[int] = a.intersection(s)\nlet k: bool = a.isdisjoint(s)\n")));
        assert!(err(&format!("{fs}let x = a[0]\n")));
        assert!(err("let a: frozenset = frozenset([1])\n"));
    }

    /// クラス側のディスクリプタ（タスク 1-12）: 読んだ値は `__get__` の戻り値の型、インスタンス経由の代入は
    /// `__set__` の値の型、クラス経由の代入（`C.x = d`）はメンバーの宣言型（ディスクリプタのクラス）と照合する。
    /// `const` に置いたデータディスクリプタへの代入は `__set__` を呼ぶので不変の誤りにしない。
    #[test]
    fn class_level_descriptors_are_typed_by_get_and_set() {
        let classes = concat!(
            "class F:\n",
            "    fn __get__(self, obj: Any, owner: Any = None) -> float:\n",
            "        return 1.5\n",
            "    fn __set__(self, mut obj: Any, value: float) -> None:\n",
            "        pass\n",
            "class G:\n",
            "    fn __get__(self, obj: Any, owner: Any = None) -> str:\n",
            "        return \"g\"\n",
            "class T:\n",
            "    mut c: float\n",
            "    static mut f: F = F()\n",
            "    const k: F = F()\n",
            "    const g: G = G()\n",
            "mut t = T(0.0)\n",
        );
        assert!(ok(&format!("{classes}let x: float = t.f\nlet y: str = t.g\nlet z: float = T.f\n")));
        assert!(err(&format!("{classes}let x: int = t.f\n")));
        assert!(ok(&format!("{classes}t.f = 2.5\nt.k = 3.0\n")));
        assert!(err(&format!("{classes}t.f = \"hot\"\n")));
        assert!(ok(&format!("{classes}T.f = F()\n")));
        assert!(err(&format!("{classes}T.f = 2.5\n")));
        assert!(ok(&format!("{classes}let n: int = t.g.upper().find(\"G\")\n")));
    }

    /// `except m.Err:`（タスク 4-3）も知らない名前なら「腕が永久に死ぬ」誤り。
    #[test]
    fn dotted_except_type_must_be_known() {
        let errs = check("try:\n    pass\nexcept nosuch.Err as e:\n    pass\n");
        assert!(
            errs.iter().any(|e| matches!(e.kind, TypeErrorKind::ExceptNotError { .. })),
            "{errs:?}"
        );
    }

    /// 組み込みの例外は `args`（組）を持つ（タスク 4-2）。`Error` を実装しただけの利用者の例外は持たない。
    #[test]
    fn builtin_exceptions_have_args() {
        assert!(ok("let e = ValueError(\"a\", 1)\nlet n = len(e.args)\n"));
        assert!(err("let e = ValueError(\"a\")\nlet n: int = e.args\n"));
        let src = concat!(
            "class MyErr(Error):\n",
            "    fn __init__(mut self, message: str) -> None:\n",
            "        self.message = message\n",
            "let m = MyErr(\"m\")\n",
            "let x = m.args\n",
        );
        assert!(err(src), "{:?}", check(src));
    }

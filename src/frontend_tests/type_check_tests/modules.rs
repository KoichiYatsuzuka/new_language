// type_check_tests/modules.rs — モジュールの型の名前空間（フェーズ10 10-8）。
//
// ⚠ 以前は型検査がクラスを素の名前だけで集めていたので、import したモジュールのクラスとメインの同名クラスが
//   混ざった（`t.Tag(1).who()` がメインの `Tag.who` の型を返し、偽の静的エラーになっていた）。
//   モジュールで宣言した型は `m.Tag` の名前で扱う。

use super::*;

    /// `import m as alias`（本体は `module_src`）を先頭に置いたプログラムを検査する。
    fn check_with_module(module_src: &str, alias: Option<&str>, main_src: &str) -> Vec<StaticTypeError> {
        use crate::ast::Stmt;
        let body = Parser::new(Lexer::new(module_src, "").tokenize(), None).parse_program().expect("parse module");
        let mut stmts = vec![Stmt::Import {
            lang: "ar".to_string(),
            module: vec!["m".to_string()],
            source_module: None,
            alias: alias.map(str::to_string),
            body,
            origin: crate::ast::ImportOrigin::default(),
            bind: Some(crate::ast::ImportBind {
                name: alias.unwrap_or("m").to_string(),
                module: vec!["m".to_string()],
            }),
        }];
        stmts.extend(Parser::new(Lexer::new(main_src, "").tokenize(), None).parse_program().expect("parse main"));
        TypeChecker::check(&stmts)
    }

    const TAGS: &str = concat!(
        "class Tag:\n",
        "    mut v: int\n",
        "    fn who(self) -> int:\n",
        "        return 1\n",
        "fn make(let n: int) -> Tag:\n",
        "    return Tag(n)\n",
    );

    const MAIN_TAG: &str = concat!(
        "class Tag:\n",
        "    mut v: int\n",
        "    fn who(self) -> str:\n",
        "        return \"main\"\n",
    );

    /// 10-8 の再現: モジュールの `Tag` とメインの `Tag` を取り違えない。
    #[test]
    fn module_class_is_not_the_main_class_of_the_same_name() {
        let errs = check_with_module(
            TAGS,
            Some("t"),
            &format!("{MAIN_TAG}let a: str = Tag(1).who()\nlet b: int = t.Tag(1).who()\n"),
        );
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// 取り違えは誤り（モジュールの `Tag` をメインの `Tag` へ）。
    #[test]
    fn module_class_into_main_class_err() {
        let errs = check_with_module(TAGS, Some("t"), &format!("{MAIN_TAG}let z: Tag = t.make(1)\n"));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].to_string().contains("m.Tag"), "{errs:?}");
    }

    /// 型注釈に別名つきの `t.Tag` を書ける。モジュールの関数の結果の型も `m.Tag`。
    #[test]
    fn dotted_annotation_names_the_module_class() {
        let errs = check_with_module(
            TAGS,
            Some("t"),
            "fn f(let x: t.Tag) -> int:\n    return x.who()\nlet r: int = f(t.make(1))\nlet s: str = f(t.make(1))\n",
        );
        assert_eq!(errs.len(), 1, "{errs:?}");
    }

    /// `import m` だけでモジュールの名前を修飾なしで書くのは未定義（10-12 の残りの穴）。
    #[test]
    fn unqualified_module_name_is_undefined() {
        let errs = check_with_module(TAGS, None, "let t = make(1)\n");
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UndefinedName { .. })), "{errs:?}");
    }

    // ── CPython と同じ名前空間（2026-10-02）──────────────────────────────────
    //
    // ⚠ フェーズ 2 では「再エクスポートしない」だったが、フェーズ 4 で CPython 準拠に改めた:
    //   モジュールの中の import の束縛もメンバー、`import a.b` は `a` を束縛し、サブモジュールは親の属性。
    //   無い名前（宣言・再エクスポート・サブモジュールのどれでもない）だけが誤り。

    fn parse(src: &str) -> Vec<crate::ast::Stmt> {
        Parser::new(Lexer::new(src, "").tokenize(), None).parse_program().expect("parse")
    }

    /// `import <module>`（束縛は `bind`。`None` はパーサが足すパッケージの連鎖の文）。
    fn import_bound(module: &[&str], body: Vec<crate::ast::Stmt>, bind: Option<(&str, &[&str])>) -> crate::ast::Stmt {
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        crate::ast::Stmt::Import {
            lang: "ar".to_string(),
            module: v(module),
            source_module: None,
            alias: None,
            body,
            origin: crate::ast::ImportOrigin::default(),
            bind: bind.map(|(name, m)| crate::ast::ImportBind { name: name.to_string(), module: v(m) }),
        }
    }

    fn import_stmt(module: &str, body: Vec<crate::ast::Stmt>) -> crate::ast::Stmt {
        import_bound(&[module], body, Some((module, &[module])))
    }

    fn from_stmt(module: &str, name: &str, body: Vec<crate::ast::Stmt>) -> crate::ast::Stmt {
        crate::ast::Stmt::FromImport {
            lang: "ar".to_string(),
            module: vec![module.to_string()],
            source_module: None,
            names: vec![(name.to_string(), None)],
            body,
            origin: crate::ast::ImportOrigin::default(),
        }
    }

    fn inner_body() -> Vec<crate::ast::Stmt> {
        parse("class Item:\n    mut v: int\nlet LABEL = \"inner\"\n")
    }

    /// `inner` を import して使うモジュール m の本体。
    fn reexporting_module() -> Vec<crate::ast::Stmt> {
        let inner = inner_body();
        let mut body = vec![import_stmt("inner", inner.clone()), from_stmt("inner", "Item", inner)];
        body.extend(parse("fn make(let v: int) -> Item:\n    return Item(v)\nlet OWN = 1\n"));
        body
    }

    /// `m.inner`（m が import したモジュール）は m のメンバー（再エクスポート・CPython と同じ）。
    #[test]
    fn module_member_imported_by_the_module_is_a_member() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let a = m.OWN\nlet x: str = m.inner.LABEL\nlet y = m.Item\n"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// 本当に無い名前は誤り。
    #[test]
    fn module_member_that_does_not_exist_is_err() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let x = m.nothing\n"));
        let errs = TypeChecker::check(&stmts);
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::ModuleHasNoMember { member, .. } if member == "nothing")),
            "{errs:?}"
        );
    }

    /// `from m import Item`（m が `from inner import Item` した名前）も取り込める（再エクスポート）。
    #[test]
    fn from_import_of_a_name_the_module_imported_is_ok() {
        let m = reexporting_module();
        let mut stmts = vec![import_stmt("m", m.clone()), from_stmt("m", "Item", m)];
        stmts.extend(parse("let it: Item = m.make(1)\n"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// m 自身の宣言は今までどおり `from m import ..` できる。
    #[test]
    fn from_import_of_an_own_name_is_ok() {
        let m = reexporting_module();
        let stmts = vec![import_stmt("m", m.clone()), from_stmt("m", "make", m)];
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// 無い名前の `from … import` は誤り。
    #[test]
    fn from_import_of_a_missing_name_is_err() {
        let m = reexporting_module();
        let stmts = vec![import_stmt("m", m.clone()), from_stmt("m", "Nothing", m)];
        let errs = TypeChecker::check(&stmts);
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::CannotImportName { name, .. } if name == "Nothing")),
            "{errs:?}"
        );
    }

    /// import していない（束縛していない）モジュール（`inner`）の型は、型の表に在っても注釈に書けない。
    #[test]
    fn type_of_a_module_not_imported_here_is_err() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let t: inner.Item = m.make(1)\n"));
        let errs = TypeChecker::check(&stmts);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(
            matches!(&errs[0].kind, TypeErrorKind::UnimportedModuleType { module, alias: None, .. } if module == "inner"),
            "{errs:?}"
        );
    }

    /// 自分でも import すれば書ける（同じ型）。
    #[test]
    fn type_of_a_module_imported_here_is_ok() {
        let mut stmts = vec![import_stmt("m", reexporting_module()), import_stmt("inner", inner_body())];
        stmts.extend(parse("let t: inner.Item = m.make(1)\n"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// `import pkg.mod` は `pkg` を束縛し、`pkg.mod.f()` を型でたどれる（CPython 準拠）。
    /// パーサはパッケージ `pkg` を読み込む文（束縛しない）を手前に足す。
    fn package_program(main_src: &str) -> Vec<crate::ast::Stmt> {
        let pkg = parse("let NAME = \"pkg\"\n");
        let module = parse("fn f(let n: int) -> int:\n    return n\nclass Tag:\n    mut v: int\n");
        let mut stmts = vec![
            import_bound(&["pkg"], pkg, None),
            import_bound(&["pkg", "mod"], module, Some(("pkg", &["pkg"]))),
        ];
        stmts.extend(parse(main_src));
        stmts
    }

    #[test]
    fn dotted_import_binds_the_package_and_types_the_chain() {
        let errs = TypeChecker::check(&package_program(
            "let a: int = pkg.mod.f(1)\nlet n: str = pkg.NAME\nlet t: pkg.mod.Tag = pkg.mod.Tag(1)\nlet b: str = pkg.mod.f(2)\n",
        ));
        assert_eq!(errs.len(), 1, "{errs:?}");
    }

    #[test]
    fn dotted_import_missing_member_of_the_submodule_is_err() {
        let errs = TypeChecker::check(&package_program("let x = pkg.mod.nothing\n"));
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::ModuleHasNoMember { module, .. } if module == "pkg.mod")),
            "{errs:?}"
        );
    }

    #[test]
    fn dotted_import_does_not_bind_the_last_segment() {
        let errs = TypeChecker::check(&package_program("let x = mod.f(1)\n"));
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UndefinedName { .. })), "{errs:?}");
    }

    // ── import[py] のクラス（python_builtins_plan.md の 6 節・2026-10-01）──────────────
    //
    // ⚠ 以前は Python のモジュールのクラスを素の名前（`Dog`）で登録していたので、`a.Dog` /
    //   `zoo.Dog` が型の名前として通らなかった（`d is a.Dog` が `unknown type`・`let x: a.Dog` が
    //   `not a known type`）。Arrow のモジュールと同じく `zoo.Dog` の名前で扱う。
    // ⚠ 変換器（`python_converter`）が `native` 限定なので、このまとまりも `native` 限定。

    /// `import[py] zoo as alias`（本体は Python の `py_src` を変換したもの）を先頭に置いたプログラムを検査する。
    #[cfg(feature = "native")]
    fn check_with_py_module(py_src: &str, alias: Option<&str>, main_src: &str) -> Vec<StaticTypeError> {
        use crate::ast::Stmt;
        let body = crate::python_converter::convert_python_source(py_src, "zoo.py").expect("convert");
        let mut stmts = vec![Stmt::Import {
            lang: "py".to_string(),
            module: vec!["zoo".to_string()],
            source_module: None,
            alias: alias.map(str::to_string),
            body,
            origin: crate::ast::ImportOrigin::default(),
            bind: Some(crate::ast::ImportBind {
                name: alias.unwrap_or("zoo").to_string(),
                module: vec!["zoo".to_string()],
            }),
        }];
        stmts.extend(Parser::new(Lexer::new(main_src, "").tokenize(), None).parse_program().expect("parse main"));
        TypeChecker::check(&stmts)
    }

    #[cfg(feature = "native")]
    const ZOO: &str = "class Animal:\n    pass\n\nclass Dog(Animal):\n    pass\n\ndef make():\n    return Dog()\n";

    /// 別名つき・別名なしの修飾名が、型の判定（`is` / `mustbe`）と型注釈に書ける。
    #[cfg(feature = "native")]
    #[test]
    fn py_module_class_is_named_by_the_module() {
        let main = concat!(
            "let d = z.Dog()\n",
            "let a = d is z.Dog\n",
            "let b = d is z.Animal\n",
            "let x: z.Dog = d\n",
            "let y = d mustbe z.Dog\n",
        );
        let errs = check_with_py_module(ZOO, Some("z"), main);
        assert!(errs.is_empty(), "{errs:?}");
        let errs = check_with_py_module(ZOO, None, "let d = zoo.Dog()\nlet a = d is zoo.Dog\nlet x: zoo.Dog = d\n");
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// `from zoo import[py] Dog` の素の名前も従来どおり通る（同じ `zoo.Dog` を指す）。
    #[cfg(feature = "native")]
    #[test]
    fn py_from_import_class_names_the_same_type() {
        use crate::ast::Stmt;
        // ⚠ パーサに `from zoo import[py] Dog` を読ませるとファイルを探しに行くので、文を組み立てて渡す。
        let body = crate::python_converter::convert_python_source(ZOO, "zoo.py").expect("convert");
        let mut stmts = vec![
            Stmt::Import {
                lang: "py".to_string(),
                module: vec!["zoo".to_string()],
                source_module: None,
                alias: Some("z".to_string()),
                body: body.clone(),
                origin: crate::ast::ImportOrigin::default(),
                bind: Some(crate::ast::ImportBind { name: "z".to_string(), module: vec!["zoo".to_string()] }),
            },
            Stmt::FromImport {
                lang: "py".to_string(),
                module: vec!["zoo".to_string()],
                source_module: None,
                names: vec![("Dog".to_string(), None)],
                body,
                origin: crate::ast::ImportOrigin::default(),
            },
        ];
        let main = "let d = Dog()\nlet a = d is Dog\nlet x: z.Dog = d\n";
        stmts.extend(Parser::new(Lexer::new(main, "").tokenize(), None).parse_program().expect("parse main"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// メインの同名クラスと取り違えない（以前は素の名前が混ざっていた）。
    #[cfg(feature = "native")]
    #[test]
    fn py_module_class_is_not_the_main_class_of_the_same_name() {
        let errs = check_with_py_module(ZOO, Some("z"), "class Dog:\n    mut v: int\nlet x: Dog = z.Dog()\n");
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].to_string().contains("zoo.Dog"), "{errs:?}");
    }

    /// import 先を読めていない（`ImportOrigin::unresolved`・エディタ）とき、レジストリに無いクラス同士の受け渡しを
    /// 誤りにしない（基底が分からない・`compare_wasm_frontend` が見つけたエディタだけの偽の誤り）。
    /// ⚠ 「読めていない」の印は `origin.unresolved`（`TypeChecker::has_unloaded_import`）。`import[py]` の本体が空なのは
    ///   本当に空のファイルなので、それだけでは読めていないことにならない。
    #[test]
    fn unknown_module_classes_are_not_a_mismatch_when_the_registry_is_incomplete() {
        use crate::ast::Stmt;
        let mut stmts = vec![Stmt::Import {
            lang: "py".to_string(),
            module: vec!["zoo".to_string()],
            source_module: None,
            alias: Some("z".to_string()),
            body: Vec::new(),
            origin: crate::ast::ImportOrigin { unresolved: true, ..Default::default() },
            bind: Some(crate::ast::ImportBind { name: "z".to_string(), module: vec!["zoo".to_string()] }),
        }];
        let main = "fn f(let a: z.Animal) -> int:\n    return 1\nfn g(let d: z.Dog) -> int:\n    return f(d)\n";
        stmts.extend(Parser::new(Lexer::new(main, "").tokenize(), None).parse_program().expect("parse main"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// 負の対照: 本体を読めていて派生でなければ、従来どおり誤り。
    #[cfg(feature = "native")]
    #[test]
    fn unrelated_py_module_classes_are_still_a_mismatch() {
        let zoo = "class Animal:\n    pass\n\nclass Dog:\n    pass\n";
        let main = "fn f(let a: z.Animal) -> int:\n    return 1\nfn g(let d: z.Dog) -> int:\n    return f(d)\n";
        let errs = check_with_py_module(zoo, Some("z"), main);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(matches!(&errs[0].kind, TypeErrorKind::CallArgTypeMismatch { .. }), "{errs:?}");
    }

    /// 存在しない名前は従来どおり弾く（修飾名なら何でも通すわけではない）。
    #[cfg(feature = "native")]
    #[test]
    fn py_module_unknown_class_is_still_an_error() {
        let errs = check_with_py_module(ZOO, Some("z"), "let d = z.Dog()\nlet a = d is z.Cat\n");
        assert!(errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::UnknownGuardType { .. })), "{errs:?}");
    }

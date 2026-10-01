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

    // ── 再エクスポートしない（2026-10-02）────────────────────────────────────
    //
    // ⚠⚠ 以前は 3 つの経路で、import したモジュールが import したものが読めた:
    //   `m.inner`（名前空間）・`from m import Item`・`let t: inner.Item`（型の表が全体で 1 つ）。

    fn parse(src: &str) -> Vec<crate::ast::Stmt> {
        Parser::new(Lexer::new(src, "").tokenize(), None).parse_program().expect("parse")
    }

    fn import_stmt(module: &str, body: Vec<crate::ast::Stmt>) -> crate::ast::Stmt {
        crate::ast::Stmt::Import {
            lang: "ar".to_string(),
            module: vec![module.to_string()],
            source_module: None,
            alias: None,
            body,
            origin: crate::ast::ImportOrigin::default(),
        }
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

    /// `inner` を import して使うモジュール m の本体。
    fn reexporting_module() -> Vec<crate::ast::Stmt> {
        let inner = parse("class Item:
    mut v: int
let LABEL = \"inner\"
");
        let mut body = vec![import_stmt("inner", inner.clone()), from_stmt("inner", "Item", inner)];
        body.extend(parse("fn make(let v: int) -> Item:
    return Item(v)
let OWN = 1
"));
        body
    }

    /// `m.inner`（m が import したモジュール）は m のメンバーではない。
    #[test]
    fn module_member_imported_by_the_module_is_err() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let a = m.OWN
let x = m.inner.LABEL
"));
        let errs = TypeChecker::check(&stmts);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(
            matches!(&errs[0].kind, TypeErrorKind::ModuleHasNoMember { member, imported: true, .. } if member == "inner"),
            "{errs:?}"
        );
    }

    /// 本当に無い名前も誤り（補足は付かない）。
    #[test]
    fn module_member_that_does_not_exist_is_err() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let x = m.nothing
"));
        let errs = TypeChecker::check(&stmts);
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::ModuleHasNoMember { imported: false, .. })),
            "{errs:?}"
        );
    }

    /// `from m import Item`（m が `from inner import Item` しただけ）は誤り。
    #[test]
    fn from_import_of_a_name_the_module_imported_is_err() {
        let m = reexporting_module();
        let stmts = vec![import_stmt("m", m.clone()), from_stmt("m", "Item", m)];
        let errs = TypeChecker::check(&stmts);
        assert!(
            errs.iter().any(|e| matches!(&e.kind, TypeErrorKind::CannotImportName { name, imported: true, .. } if name == "Item")),
            "{errs:?}"
        );
    }

    /// m 自身の宣言は今までどおり `from m import ..` できる。
    #[test]
    fn from_import_of_an_own_name_is_ok() {
        let m = reexporting_module();
        let stmts = vec![import_stmt("m", m.clone()), from_stmt("m", "make", m)];
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

    /// import していないモジュール（`inner`）の型は、型の表に在っても注釈に書けない。
    #[test]
    fn type_of_a_module_not_imported_here_is_err() {
        let mut stmts = vec![import_stmt("m", reexporting_module())];
        stmts.extend(parse("let t: inner.Item = m.make(1)
"));
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
        let inner = parse("class Item:
    mut v: int
let LABEL = \"inner\"
");
        let mut stmts = vec![import_stmt("m", reexporting_module()), import_stmt("inner", inner)];
        stmts.extend(parse("let t: inner.Item = m.make(1)
"));
        let errs = TypeChecker::check(&stmts);
        assert!(errs.is_empty(), "{errs:?}");
    }

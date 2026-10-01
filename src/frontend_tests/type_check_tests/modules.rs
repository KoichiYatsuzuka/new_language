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

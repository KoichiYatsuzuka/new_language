// tests/modules.rs — `import` の名前空間の分離（2026-09-26）。
//
// ⚠⚠ 以前は `import m` のたびに、モジュールの最上位の名前を**呼び出し側の大域へ流し込んで**いた
// （モジュールの関数が同じモジュールの名前を引くための仕組み）。呼び出し側の名前空間が侵され、
// 同名の `const` が再宣言になる・同名の `fn` が多重定義として合成されて乗っ取られる等が起きた。
// 今は関数が定義したモジュールの大域を持ち、呼び出しの間だけその大域へ差し替える。

use super::*;

/// `import m`（本体は `module_src`）を先頭に置いたプログラムを、本番と同じ配線で走らせる。
///
/// ⚠ 本体はファイルから読まず、パース済みの本体を `Stmt::Import` に入れて渡す
///   （パーサが import 先を読み込んだ後の形と同じ）。
fn run_with_module(module_src: &str, main_src: &str) -> Result<Interpreter, String> {
    let body = Parser::new(Lexer::new(module_src, "").tokenize(), None).parse_program()?;
    let mut stmts = vec![Stmt::Import {
        lang: "ar".to_string(),
        module: vec!["m".to_string()],
        source_module: None,
        alias: None,
        body,
        origin: crate::ast::ImportOrigin::default(),
        bind: Some(crate::ast::ImportBind { name: "m".to_string(), module: vec!["m".to_string()] }),
    }];
    let mut parser = Parser::new(Lexer::new(main_src, "").tokenize(), None);
    stmts.extend(parser.parse_program()?);
    run_prepared(stmts, &parser)
}

/// 組み立て済みのプログラムを、本番と同じ配線（展開 → 解決 → 注釈の注入）で走らせる。
/// `parser` は node-id の採番と trait 表を展開器へ渡すためのもの。
fn run_prepared(stmts: Vec<Stmt>, parser: &Parser) -> Result<Interpreter, String> {
    // 本番と同じく展開を先に済ませる（テンプレートの具体化・D36）。
    let mut stmts =
        crate::meta_expand::expand_program(stmts, parser.node_counter(), parser.known_traits())?;
    let (_errors, _warnings, annotations) = super::super::resolver::resolve_and_annotate(&mut stmts);
    let mut interp = Interpreter::new();
    interp.wire_resolution(
        annotations,
        super::super::resolver::toplevel_declared_globals(&stmts),
        crate::interpreter::GlobalsMode::Replace,
    );
    for stmt in &stmts {
        if let Err(e) = interp.exec(stmt) {
            // 関数の中で起きた失敗は例外として上がってくる（本文は例外の側にある）。
            return Err(match interp.take_current_exception() {
                Some(r) if e == crate::interpreter::RAISE_SENTINEL => Interpreter::format_error_report(&r),
                _ => e,
            });
        }
    }
    Ok(interp)
}

const MODULE: &str = "\
const SCALE = 10

fn scaled(let n: int) -> int:
    return n * SCALE

class Square:
    mut side: int

    fn area(self) -> int:
        return scaled(self.side * self.side)

let SAMPLE = Square(2).area()
";

/// 呼び出し側の `let` / `fn` がモジュールの同名の名前とぶつからない（再宣言にも多重定義にもならない）。
#[test]
fn a_caller_name_does_not_collide_with_a_module_name() {
    let interp = run_with_module(
        MODULE,
        "let SCALE = 3\nfn scaled(let n: int) -> int:\n    return n + 1000\nlet mine = scaled(4)\nlet theirs = m.scaled(4)\n",
    )
    .expect("run");
    assert!(matches!(interp.get_val("SCALE"), Some(Value::Int(3))));
    assert!(matches!(interp.get_val("mine"), Some(Value::Int(1004))), "呼び出し側の関数が乗っ取られている");
    assert!(matches!(interp.get_val("theirs"), Some(Value::Int(40))));
}

/// モジュールの名前は修飾なしでは見えない。
#[test]
fn a_module_name_is_not_visible_unqualified() {
    let err = run_with_module(MODULE, "let x = scaled(2)\n").err().expect("NameError のはず");
    assert!(err.contains("'scaled' is not defined"), "{err}");
}

/// 逆向き: モジュールの関数は呼び出し側の名前を引けない。
#[test]
fn a_module_function_does_not_see_caller_names() {
    let err = run_with_module("fn peek() -> int:\n    return SECRET\n", "const SECRET = 1\nlet x = m.peek()\n")
        .err()
        .expect("NameError のはず");
    assert!(err.contains("'SECRET' is not defined"), "{err}");
}

/// モジュールのメソッドはモジュールの名前を引く。読み込みの最中（モジュールの最上位）から呼んでも同じ
/// （以前は流し込みが終わる前なので `NameError` だった）。
#[test]
fn a_module_method_sees_module_names_even_while_loading() {
    let interp = run_with_module(MODULE, "let a = m.Square(3).area()\nlet s = m.SAMPLE\n").expect("run");
    assert!(matches!(interp.get_val("a"), Some(Value::Int(90))));
    assert!(matches!(interp.get_val("s"), Some(Value::Int(40))));
}

/// モジュールの `mut` 変数の変更が `m.x` に映る（フェーズ10 10-11）。
/// ⚠ 以前は名前空間が import した時点の写しで、`m.bump()` を何度呼んでも `m.count` は 0 のままだった。
#[test]
fn a_module_mut_variable_is_read_live() {
    let module = "mut count = 0\nlet STEP = 1\nfn bump() -> None:\n    count += STEP\n";
    let interp = run_with_module(
        module,
        concat!(
            "let before = m.count\n",
            "m.bump()\n",
            "m.bump()\n",
            "fn read() -> int:\n",
            "    return m.count\n",
            "let after = m.count\n",
            "let inside = read()\n",
            "let step = m.STEP\n",
        ),
    )
    .expect("run");
    assert!(matches!(interp.get_val("before"), Some(Value::Int(0))));
    assert!(matches!(interp.get_val("after"), Some(Value::Int(2))));
    assert!(matches!(interp.get_val("inside"), Some(Value::Int(2))));
    assert!(matches!(interp.get_val("step"), Some(Value::Int(1))));
}

/// モジュールのクラスは `m.Tag` の名前でも判定できる（`is` / `mustbe`・フェーズ10 10-8）。
/// ⚠ 型検査が付ける実行時の検査（`CheckBefore`）にも `m.Tag` が来る。表示名（`Tag`）は変えない。
#[test]
fn a_module_class_matches_its_qualified_name() {
    let module = "class Tag:\n    mut v: int\nfn make(let n: int) -> Tag:\n    return Tag(n)\n";
    let interp = run_with_module(
        module,
        concat!(
            "class Tag:\n",
            "    mut v: int\n",
            "let x = m.make(1)\n",
            "let q = x is m.Tag\n",
            "let mine = x is Tag\n",
            "let y = x mustbe m.Tag\n",
            "let n = y.v\n",
        ),
    )
    .expect("run");
    assert!(matches!(interp.get_val("q"), Some(Value::Bool(true))));
    assert!(matches!(interp.get_val("n"), Some(Value::Int(1))));
    // ⚠ 素の名前での判定は従来どおり（実行時は表示名でも当たる。静的には別の型として弾く）。
    assert!(matches!(interp.get_val("mine"), Some(Value::Bool(true))));
}

// ── CPython と同じ名前空間（2026-10-02）────────────────────────────────────
//
// ⚠ フェーズ 2 では「再エクスポートしない」（モジュールの中の import の束縛を名前空間から外す）だったが、
//   フェーズ 4 で CPython 準拠に改めた。モジュールが import した名前もメンバーで、サブモジュールは
//   親パッケージの属性になる（`Interpreter::attach_submodule`）。

const INNER: &str = "class Item:\n    let v: int\n    fn __init__(mut self, let v: int):\n        self.v = v\nlet LABEL = \"inner\"\n";

fn import_stmt(module: &[&str], body: Vec<Stmt>, bind: Option<(&str, &[&str])>) -> Stmt {
    let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Stmt::Import {
        lang: "ar".to_string(),
        module: v(module),
        source_module: None,
        alias: None,
        body,
        origin: crate::ast::ImportOrigin::default(),
        bind: bind.map(|(name, m)| crate::ast::ImportBind { name: name.to_string(), module: v(m) }),
    }
}

fn from_stmt(module: &str, name: &str, body: Vec<Stmt>) -> Stmt {
    Stmt::FromImport {
        lang: "ar".to_string(),
        module: vec![module.to_string()],
        source_module: None,
        names: vec![(name.to_string(), None)],
        body,
        origin: crate::ast::ImportOrigin::default(),
    }
}

/// `inner` を import する外側のモジュール m の本体（`import inner` / `from inner import Item` ＋ `outer_src`）。
fn outer_module_body(outer_src: &str) -> Vec<Stmt> {
    let inner = Parser::new(Lexer::new(INNER, "").tokenize(), None).parse_program().expect("parse inner");
    let mut body = vec![
        import_stmt(&["inner"], inner.clone(), Some(("inner", &["inner"]))),
        from_stmt("inner", "Item", inner),
    ];
    body.extend(Parser::new(Lexer::new(outer_src, "").tokenize(), None).parse_program().expect("parse outer"));
    body
}

const OUTER: &str = "fn make(let v: int) -> Item:\n    return Item(v)\nfn label() -> str:\n    return inner.LABEL\n";

/// 外側のモジュールが import した `inner` は、外側の名前空間のメンバー（再エクスポート）。
#[test]
fn a_module_reexports_a_module_it_imports() {
    let mut stmts = vec![import_stmt(&["m"], outer_module_body(OUTER), Some(("m", &["m"])))];
    let mut parser = Parser::new(Lexer::new("let x = m.inner.LABEL\n", "").tokenize(), None);
    stmts.extend(parser.parse_program().unwrap());
    let interp = run_prepared(stmts, &parser).expect("run");
    assert!(matches!(interp.get_val("x"), Some(Value::Str(s)) if s.as_ref() == "inner"));
}

/// 外側のモジュールが `from inner import Item` した `Item` も取り込める（再エクスポート）。
#[test]
fn a_module_reexports_a_name_it_imports() {
    let outer = outer_module_body(OUTER);
    let mut stmts = vec![
        import_stmt(&["m"], outer.clone(), Some(("m", &["m"]))),
        from_stmt("m", "Item", outer),
    ];
    let mut parser = Parser::new(Lexer::new("let v = Item(3).v\n", "").tokenize(), None);
    stmts.extend(parser.parse_program().unwrap());
    let interp = run_prepared(stmts, &parser).expect("run");
    assert!(matches!(interp.get_val("v"), Some(Value::Int(3))));
}

/// 外側のモジュールの関数は、外側が import した名前（`inner` / `Item`）を使える。
#[test]
fn a_module_still_uses_what_it_imports() {
    let mut stmts = vec![import_stmt(&["m"], outer_module_body(OUTER), Some(("m", &["m"])))];
    let mut parser = Parser::new(Lexer::new("let v = m.make(7).v\nlet l = m.label()\n", "").tokenize(), None);
    stmts.extend(parser.parse_program().unwrap());
    let interp = run_prepared(stmts, &parser).expect("run");
    assert!(matches!(interp.get_val("v"), Some(Value::Int(7))));
    assert!(matches!(interp.get_val("l"), Some(Value::Str(s)) if s.as_ref() == "inner"));
}

/// `import pkg.mod` は `pkg` を束縛し、`pkg.mod` はパッケージの属性（`attach_submodule`・CPython と同じ）。
/// パーサはパッケージ `pkg` を読み込む文（束縛しない）を手前に足す。
#[test]
fn a_dotted_import_binds_the_package_and_attaches_the_submodule() {
    let pkg = Parser::new(Lexer::new("let NAME = \"pkg\"\n", "").tokenize(), None).parse_program().unwrap();
    let module = Parser::new(Lexer::new("let X = 5\n", "").tokenize(), None).parse_program().unwrap();
    let mut stmts = vec![
        import_stmt(&["pkg"], pkg, None),
        import_stmt(&["pkg", "mod"], module, Some(("pkg", &["pkg"]))),
    ];
    let mut parser = Parser::new(Lexer::new("let x = pkg.mod.X\nlet n = pkg.NAME\n", "").tokenize(), None);
    stmts.extend(parser.parse_program().unwrap());
    let interp = run_prepared(stmts, &parser).expect("run");
    assert!(matches!(interp.get_val("x"), Some(Value::Int(5))));
    assert!(matches!(interp.get_val("n"), Some(Value::Str(s)) if s.as_ref() == "pkg"));
    // 末尾の `mod` は束縛されない。
    assert!(interp.get_val("mod").is_none());
}

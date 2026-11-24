use super::*;

/// ソースを展開して、残った文の種別名を並べたものを返す。
fn expand_kinds(src: &str) -> Result<Vec<&'static str>, String> {
    let out = expand(src)?;
    Ok(out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect())
}

/// パースして、文の列と**そのパーサが使った node-id カウンタ**を返す。
///
/// ⚠ カウンタを展開器へ渡すのが要点（設計書 §0.3 / タスク 2-1）。
type Parsed = (
    Vec<Stmt>,
    std::rc::Rc<std::cell::Cell<u32>>,
    std::collections::HashMap<String, crate::parser::TraitInfo>,
);

fn parse(src: &str) -> Result<Parsed, String> {
    let tokens = crate::lexer::Lexer::new(src, "").tokenize();
    let mut parser = crate::parser::Parser::new(tokens, None);
    let stmts = parser.parse_program()?;
    Ok((stmts, parser.node_counter(), parser.known_traits()))
}

fn expand(src: &str) -> Result<Vec<Stmt>, String> {
    let (stmts, counter, traits) = parse(src)?;
    expand_program(stmts, counter, traits)
}

/// メタ関数の定義は**登録されて AST から消える**（設計書 §1.7）。
/// ⚠ 消えないと、展開後の AST に実行できない文が残る。
#[test]
fn metafn_definitions_are_removed_from_the_ast() {
    let kinds = expand_kinds(
        "exprconst !fn place_it() -> None:\n    quote code:\n\nprint(1)\n",
    )
    .expect("expand");
    assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
}

/// 配置メタ関数の呼び出しが、`quote` した `Code` の中身で置き換わる。
#[test]
fn a_placing_metafn_call_is_replaced_by_its_code() {
    let out = expand(
        "exprconst !fn place_it() -> None:\n    quote code:\n        let placed = 1\n\nplace_it()\n",
    )
    .expect("expand");
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], Stmt::Let(name, _, _, _) if name == "placed"),
        "置かれたのは `let placed = 1`: {:?}",
        crate::interpreter::tw_stats::stmt_kind_of(&out[0])
    );
}

/// 空の `code:` を `quote` すると**何も置かれない**（設計書 §1.2 の B-2）。
#[test]
fn quoting_an_empty_code_block_places_nothing() {
    let kinds = expand_kinds(
        "exprconst !fn place_nothing() -> None:\n    quote code:\n\nplace_nothing()\nprint(1)\n",
    )
    .expect("expand");
    assert_eq!(kinds, vec!["Expr"], "`print(1)` だけが残る");
}

/// スプライス `<! !>` が展開時の値で埋まる（設計書 §1.4）。
/// ⚠ `str` は**識別子**になる（文字列リテラルではない）。
#[test]
fn a_splice_fills_in_an_identifier() {
    let out = expand(
        "exprconst !fn bind(n: str) -> None:\n    quote code:\n        let <! n !> = 1\n\nbind(\"chosen\")\n",
    )
    .expect("expand");
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], Stmt::Let(name, _, _, _) if name == "chosen"),
        "スプライスした名前で束縛される"
    );
}

/// 置いたコードがさらにメタ関数を呼ぶ形も、同じ前向きの走査で処理される。
/// ⚠ これは**反復ではない**（§1.7 の「前方のみ参照」は保たれる）。
#[test]
fn code_placed_by_a_metafn_is_expanded_too() {
    let out = expand(
        "exprconst !fn inner() -> None:\n    quote code:\n        let deep = 1\n\nexprconst !fn outer() -> None:\n    quote code:\n        inner()\n\nouter()\n",
    )
    .expect("expand");
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Stmt::Let(name, _, _, _) if name == "deep"));
}

/// ⚠ `quote` に到達せず抜けたら**エラー**（設計書 §1.1）。
/// 黙って無視すると「メタ関数を書いたのに何も起きない」になる。
#[test]
fn a_placing_metafn_that_never_quotes_is_an_error() {
    let err = expand("exprconst !fn nothing() -> None:\n    pass\n\nnothing()\n")
        .expect_err("エラーになること");
    assert!(err.contains("without placing any `Code`"), "実際のエラー: {err}");
}

/// ⚠⚠ 展開時の `print` は **stderr** へ出す（設計書 §4.1 / タスク 3-4）。
/// stdout に混ざると `compare_python_impl` / `compare_outputs` の差分比較が壊れ、
/// 「意味論を守る唯一の網」が**黙って無効**になる。
#[test]
fn the_expander_redirects_print_to_stderr() {
    let mut ex = Expander::new(
        std::rc::Rc::new(std::cell::Cell::new(0)),
        std::collections::HashMap::new(),
    );
    assert!(!ex.interp.is_meta_expanding(), "前口上を読む前は既定のまま");
    ex.load_prelude().expect("prelude");
    assert!(
        ex.interp.is_meta_expanding(),
        "展開器のインタプリタが出力先を切り替えていない"
    );
}

/// ⚠⚠ **装飾子ファクトリ**（設計書 §1.6「装飾子にはクロージャも書ける」・タスク 4-8）。
/// `!repeat(3)` は `repeat(3)` を評価し、返ってきたクロージャを対象に適用する
/// —— Python の `@deco(args)` と同じ形。クロージャの中の `quote` はそのクロージャを抜ける
/// （ファクトリは既に返っているので、それ以外の意味は取りようがない）。
#[test]
fn a_decorator_factory_receives_its_arguments() {
    let out = expand(concat!(
        "exprconst fn tag_with(label: str):\n",
        "    fn deco(m) -> None:\n",
        "        let n = m.name + \"_\" + label\n",
        "        quote code:\n",
        "            let <! n !> = 1\n",
        "    return deco\n",
        "\n",
        "!tag_with(\"seen\")\n",
        "fn f() -> int:\n",
        "    return 1\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "f_seen")),
        "ファクトリの引数が届いていない"
    );
}

/// ⚠⚠ メタ関数に束縛した `const` は**展開時専用**なので実行時の AST から消える（タスク 4-8）。
/// 残すと実行時に**もう存在しないメタ関数の名前**を読んで `NameError` になる（実測）。
/// ⚠ `tagged` は配置メタ関数。別名で呼んでも入口の検査（3-8）を通ることもここで見ている
/// —— 旗を**名前ではなく実体で**立てているから通る。
#[test]
fn a_const_bound_to_a_metafn_works_as_a_decorator_and_vanishes() {
    let out = expand(concat!(
        "exprconst !fn tagged(m) -> None:\n",
        "    quote code:\n",
        "        let tag_seen = 1\n",
        "\n",
        "const my_deco = tagged\n",
        "\n",
        "!my_deco\n",
        "fn f() -> int:\n",
        "    return 1\n",
    ))
    .expect("expand");
    let kinds: Vec<&str> = out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect();
    assert_eq!(kinds, vec!["Let"], "`const my_deco` が残っていない・装飾子が効いていること");
}

/// ⚠ 呼べないものを装飾子に書いたら**そう言う**。
#[test]
fn a_decorator_that_is_not_callable_is_an_error() {
    let err = expand(concat!(
        "exprconst fn not_a_deco() -> int:\n",
        "    return 42\n",
        "\n",
        "!not_a_deco()\n",
        "fn f() -> int:\n",
        "    return 1\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("did not produce something callable"), "実際のエラー: {err}");
}

/// ⚠⚠ **配置メタ関数を呼んでよいのは展開器だけ**（設計書 §1.1 の表・タスク 3-8）。
/// メタ関数の本体から呼ぶと `quote` の結果はただの返り値になり、**黙って捨てられた**
/// （実測: 置いたつもりの `print("hi")` が消えて exit 0）。純粋メタ関数からも同じ。
#[test]
fn a_placing_metafn_called_from_a_metafn_is_an_error() {
    let greet = concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
    );
    let from_placing = format!(
        "{greet}exprconst !fn outer() -> None:\n    greet()\n    quote code:\n\nouter()\n"
    );
    let from_pure = format!(
        "{greet}exprconst fn helper() -> Code:\n    greet()\n    let c = code:\n    return c\n\n\
             exprconst !fn outer() -> None:\n    quote helper()\n\nouter()\n"
    );
    for src in [from_placing, from_pure] {
        let err = expand(&src).expect_err("弾かれること");
        assert!(
            err.contains("placing metafunction 'greet' can only run as a statement"),
            "実際のエラー: {err}"
        );
        assert!(err.contains("(raised by metafunction 'outer')"), "実際のエラー: {err}");
    }
}

/// ⚠ 実引数の中（式の位置）でも同じ。展開器が呼ぶのは**文として書かれた呼び出し**だけ。
#[test]
fn a_placing_metafn_used_as_an_argument_is_an_error() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "\n",
        "exprconst !fn take(c) -> None:\n",
        "    quote code:\n",
        "\n",
        "take(greet())\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("can only run as a statement")
            && err.contains("(raised while evaluating the arguments of 'take')"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ 実引数の中の `compile_error` は**文面が出る**（タスク 3-8）。
/// 以前は実引数の評価だけ例外の整形を通っておらず、番兵文字列 ` __raise__` が漏れた（実測）。
#[test]
fn compile_error_in_an_argument_is_reported_with_its_message() {
    let err = expand(concat!(
        "exprconst fn checked(n: int) -> int:\n",
        "    if n < 0:\n",
        "        compile_error(\"n must not be negative\")\n",
        "    return n\n",
        "\n",
        "exprconst !fn bind(value: int) -> None:\n",
        "    quote code:\n",
        "        let x = <! value !>\n",
        "\n",
        "bind(checked(-1))\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("n must not be negative (raised while evaluating the arguments of 'bind')"),
        "実際のエラー: {err}"
    );
    assert!(!err.contains("__raise__"), "番兵が漏れている: {err}");
}

/// ⚠⚠ 装飾子式の評価の失敗は**握り潰さない**（タスク 3-8）。
/// 以前は `.ok()` で捨てていたので「前に宣言されていない」という無関係な診断になった（実測）。
#[test]
fn a_failing_decorator_expression_is_described() {
    let err = expand(concat!(
        "exprconst fn tag_with(n: int):\n",
        "    fn deco(m) -> None:\n",
        "        quote code:\n",
        "    return deco\n",
        "\n",
        "!tag_with(1 // 0)\n",
        "fn f() -> int:\n",
        "    return 1\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("while evaluating decorator '!tag_with(...)'") && err.contains("ZeroDivisionError"),
        "実際のエラー: {err}"
    );
    assert!(!err.contains("not a metafunction declared"), "取り違えている: {err}");
}

/// ⚠ 実行時の `NameError` への書き添えは**メタ関数の名前のときだけ**（タスク 3-8）。
#[test]
fn a_runtime_name_error_on_a_removed_metafn_gets_a_note() {
    let metafns: std::collections::HashMap<String, bool> =
        [("greet".to_string(), true), ("frag".to_string(), false)].into_iter().collect();
    let placing = explain_removed_metafn("NameError: 'greet' is not defined".into(), &metafns);
    assert!(placing.contains("note: 'greet' is a placing metafunction"), "{placing}");
    let pure = explain_removed_metafn("NameError: 'frag' is not defined".into(), &metafns);
    assert!(pure.contains("note: 'frag' is a pure metafunction"), "{pure}");
    let other = explain_removed_metafn("NameError: 'other' is not defined".into(), &metafns);
    assert_eq!(other, "NameError: 'other' is not defined", "関係ない名前には書き添えない");
}

/// ⚠⚠ 宣言は**あるが**メタ情報を持たない種類に `^` を当てたら、**そう言う**（タスク 3-9）。
/// 以前は対象の種類だけを宣言表に入れていたので「前に何も宣言されていない」と誤報した（実測）。
#[test]
fn caret_on_a_metafn_says_what_it_is() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "\n",
        "exprconst !fn show(m) -> None:\n",
        "    quote code:\n",
        "\n",
        "show(^greet)\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("`^greet` is not available — 'greet' is a placing metafunction"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ `^` が**実行時に**評価されたら「宣言されていない」ではない（タスク 3-9）。
/// `print(^T)` は位置制限（1-3）を通る（呼び先がメタ関数かはパース時に分からない）ので、
/// 展開器が評価しないまま実行時に来る。以前は「前に何も宣言されていない」と誤報した（実測）。
#[test]
fn caret_evaluated_at_run_time_says_so() {
    let interp = Interpreter::new();
    let err = interp.meta_lookup_error("Marker");
    assert!(
        err.contains("can only be evaluated while the compile-time expander runs"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ 装飾子の失敗は**どの宣言のものか**を位置で言う（タスク 3-9）。
/// `Expr::Ident` は位置を持たないので、以前は位置の無い診断になっていた（実測）。
#[test]
fn a_decorator_failure_points_at_the_decorator() {
    let err = expand(concat!(
        "exprconst fn not_a_deco() -> int:\n",
        "    return 42\n",
        "\n",
        "!not_a_deco()\n",
        "fn f() -> int:\n",
        "    return 1\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("while expanding '!not_a_deco(...)' at") && err.contains("line 4, col 1"),
        "実際のエラー: {err}"
    );
}

/// ⚠ メソッド形の射影（`has_field` など）も**どの射影か**を言う（タスク 3-9）。
#[test]
fn a_method_projection_that_does_not_fit_names_itself() {
    let err = expand(concat!(
        "fn helper() -> int:\n",
        "    return 1\n",
        "\n",
        "exprconst !fn probe(m) -> None:\n",
        "    let b = m.has_field(\"x\")\n",
        "    quote code:\n",
        "\n",
        "probe(^helper)\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("`.has_field()` needs a class or trait"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ 置いたコードの構文エラーは `code:` の中の**書いた位置**を指す（タスク 5-1 で発見）。
/// 2-3 で行ごとに位置を記録していたのに、この文面では使っておらず呼び出し位置しか出なかった。
#[test]
fn a_placed_code_parse_error_points_into_the_code_block() {
    let err = expand(concat!(
        "exprconst !fn broken() -> None:\n",
        "    quote code:\n",
        "        let = 1\n",
        "\n",
        "broken()\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("inside the `code:` block") && err.contains("line 3, col 13"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ **メタ関数でない関数の中では、メタ関数を呼べない**（設計書 §1.1・2026-09-25 確定・タスク 2-11）。
/// 以前は展開されずに残り、**その関数を呼んだときに**実行時 `NameError` になった。
#[test]
fn a_placing_call_inside_a_function_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "fn work() -> None:\n",
        "    greet()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("placing metafunction 'greet' cannot be called inside function 'work'"), "実際のエラー: {err}");
}

/// ⚠ メソッドも関数。クラス本体に**文として**書いた呼び出しとは別物。
#[test]
fn a_placing_call_inside_a_method_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "class C:\n",
        "    fn m(self) -> None:\n",
        "        greet()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("cannot be called inside function 'm'"), "実際のエラー: {err}");
}

/// ⚠ 最上位でも**ブロックの中**は置き換えない。ブロックは実行時に走るが、
/// メタ関数は展開時に走る（展開時に選びたいならメタ関数の中で `const` で分岐する）。
#[test]
fn a_placing_call_inside_a_top_level_block_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "const DEBUG = True\n",
        "if DEBUG:\n",
        "    greet()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("cannot be called inside a block"), "実際のエラー: {err}");
}

/// ⚠ 配置メタ関数は値を返さない（§1.1）。
#[test]
fn a_placing_metafn_used_as_a_value_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "let x = greet()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("placing metafunction 'greet' cannot be used as a value"), "実際のエラー: {err}");
}

/// ⚠ 純粋メタ関数は `Code` を返す。通常コードからは呼べない（§1.1）。
#[test]
fn a_pure_metafn_called_from_ordinary_code_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "exprconst fn frag() -> Code:\n",
        "    let c = code:\n",
        "    return c\n",
        "\n",
        "frag()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("pure metafunction 'frag' cannot be called from ordinary code"), "実際のエラー: {err}");
}

/// ⚠ 入れ子のメタ関数の定義は許さない（§1.1・2026-09-25 確定）。
#[test]
fn a_metafn_defined_inside_a_function_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "fn outer() -> None:\n",
        "    exprconst fn inner() -> Code:\n",
        "        let c = code:\n",
        "        return c\n",
        "    print(1)\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("metafunction 'inner' is defined inside function 'outer'"), "実際のエラー: {err}");
}

/// ⚠ 関数の中の `!装飾子` も展開されない。以前は実行時まで残って落ちた。
#[test]
fn a_decorator_inside_a_function_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "exprconst !fn deco(m) -> None:\n",
        "    quote m.code()\n",
        "\n",
        "fn f() -> None:\n",
        "    !deco\n",
        "    let x = 1\n",
        "    print(x)\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("decorator '!deco' is written inside function 'f'"), "実際のエラー: {err}");
}

/// ⚠ 通常コードの `^` は展開時に止める（以前は実行時に 3-9 の文面で落ちた）。
#[test]
fn caret_in_ordinary_code_is_rejected_while_expanding() {
    let err = expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "class Marker:\n",
        "    mut id: int\n",
        "\n",
        "print(^Marker)\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("`^Marker` is written at the top level in ordinary code"), "実際のエラー: {err}");
}

/// ⚠⚠ 同じ名前の普通の束縛（ここでは仮引数）があるメタ関数は**調べない**（保守的・2-11）。
/// 見分けるにはスコープの追跡が要り、取り違えると正しいプログラムを弾くので。
#[test]
fn a_parameter_that_shadows_a_metafn_is_not_rejected() {
    expand(concat!(
        "exprconst !fn greet() -> None:\n",
        "    quote code:\n",
        "        print(\"hi\")\n",
        "\n",
        "fn shadow(greet: int) -> int:\n",
        "    return greet + 1\n",
        "\n",
        "greet()\n",
    ))
    .expect("仮引数の `greet` はメタ関数ではない");
}

/// `import` 文をディスクを読まずに組み立てて展開する（タスク 2-12 のテスト用）。
///
/// ⚠ モジュールと本体は**同じ node-id カウンタ**でパースする（パーサの `parse_sub_module` と同じ）。
fn expand_with_module(module_src: &str, imports: &[Stmt], main_src: &str) -> Result<Vec<Stmt>, String> {
    expand_with_module_after("", module_src, imports, main_src)
}

/// [`expand_with_module`] と同じだが、`import` の**前**にメインの文 `prefix_src` を置く。
fn expand_with_module_after(
    prefix_src: &str,
    module_src: &str,
    imports: &[Stmt],
    main_src: &str,
) -> Result<Vec<Stmt>, String> {
    let (body, counter, traits) = parse(module_src)?;
    let parse_main = |src: &str| {
        let tokens = crate::lexer::Lexer::new(src, "").tokenize();
        let mut parser = crate::parser::Parser::new(tokens, None);
        parser.set_node_counter(std::rc::Rc::clone(&counter));
        parser.parse_program()
    };
    let prefix = parse_main(prefix_src)?;
    let main = parse_main(main_src)?;
    let mut stmts: Vec<Stmt> = prefix;
    for imp in imports {
        let mut imp = imp.clone();
        match &mut imp {
            Stmt::Import { body: b, .. } | Stmt::FromImport { body: b, .. } => *b = body.clone(),
            _ => unreachable!(),
        }
        stmts.push(imp);
    }
    stmts.extend(main);
    expand_program(stmts, counter, traits)
}

fn import_of(alias: Option<&str>) -> Stmt {
    Stmt::Import {
        lang: "ar-auto".to_string(),
        module: vec!["m".to_string()],
        source_module: None,
        alias: alias.map(str::to_string),
        body: Vec::new(),
        origin: crate::ast::ImportOrigin::default(),
        bind: Some(crate::ast::ImportBind {
            name: alias.unwrap_or("m").to_string(),
            module: vec!["m".to_string()],
        }),
    }
}

fn from_import_of(names: &[&str]) -> Stmt {
    Stmt::FromImport {
        lang: "ar-auto".to_string(),
        module: vec!["m".to_string()],
        source_module: None,
        names: names.iter().map(|n| (n.to_string(), None)).collect(),
        body: Vec::new(),
        origin: crate::ast::ImportOrigin::default(),
    }
}

fn module_src() -> &'static str {
    concat!(
        "const LEVEL = 3\n",
        "exprconst fn helper_line() -> Code:\n",
        "    let c = code:\n",
        "        let from_module = <! LEVEL !>\n",
        "    return c\n",
        "exprconst !fn place_it() -> None:\n",
        "    quote helper_line()\n",
        "exprconst !fn tick() -> None:\n",
        "    quote code:\n",
        "        let <! gensym(\"tick\") !> = 1\n",
        "tick()\n",
    )
}

/// ⚠⚠ **別のモジュールのメタ関数も使える**（設計書 §1.7・2026-09-25 確定・タスク 2-12）。
/// `import m` なら `m.place_it()` が配置呼び出しになる。
#[test]
fn a_module_metafn_can_be_called_through_the_module_name() {
    let out = expand_with_module(module_src(), &[import_of(None)], "m.place_it()\n").expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "from_module")),
        "`m.place_it()` が置き換えられていない"
    );
}

/// ⚠ `from m import name` なら素の名前で呼べる。⚠⚠ メタ関数の名前は**実行時の import から外れる**
/// （残すと実行時に `ImportError: cannot import name` になった・実測）。
#[test]
fn a_from_imported_metafn_is_called_by_its_plain_name() {
    let out = expand_with_module(module_src(), &[from_import_of(&["place_it"])], "place_it()\n")
        .expect("expand");
    assert!(out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "from_module")));
    let Some(Stmt::FromImport { names, .. }) = out.first() else { panic!("FromImport を期待") };
    assert!(names.is_empty(), "メタ関数の名前が実行時の import に残っている: {names:?}");
}

/// ⚠⚠ モジュールのメタ関数は**自分のモジュールの名前**を引く（閉包として捕まえる）。
/// 呼び出し側に同じ名前のメタ関数があっても取り違えない（実行時のモジュール関数と同じ性質）。
#[test]
fn a_module_metafn_uses_its_own_module_names() {
    let main = concat!(
        "exprconst fn helper_line() -> Code:\n",
        "    let c = code:\n",
        "        let from_main = 1\n",
        "    return c\n",
        "\n",
        "m.place_it()\n",
    );
    let out = expand_with_module(module_src(), &[import_of(None)], main).expect("expand");
    assert!(out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "from_module")));
    assert!(!out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "from_main")));
}

/// ⚠⚠ モジュールは**初めて読み込まれるときに 1 回だけ**展開する（§1.7・2026-09-25 確定）。
/// 2 回目の import は展開済みの本体を使い回す —— モジュールの中の `gensym` が 1 回しか進まない。
#[test]
fn a_module_imported_twice_is_expanded_once() {
    let out = expand_with_module(module_src(), &[import_of(None), import_of(Some("again"))], "")
        .expect("expand");
    let names_in = |s: &Stmt| -> Vec<String> {
        let Stmt::Import { body, .. } = s else { panic!("Import を期待") };
        body.iter()
            .filter_map(|st| match st {
                Stmt::Let(n, _, _, _) => Some(n.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(names_in(&out[0]), vec!["tick__1".to_string()]);
    assert_eq!(names_in(&out[1]), vec!["tick__1".to_string()], "2 回目も同じ本体（再展開していない）");
}

/// ⚠ モジュールの中の展開エラーは**どのモジュールか**を添える。
#[test]
fn an_error_inside_a_module_names_the_module() {
    let bad = "exprconst !fn boom() -> None:\n    compile_error(\"boom\")\n    quote code:\n\nboom()\n";
    let err = expand_with_module(bad, &[import_of(None)], "").expect_err("弾かれること");
    assert!(err.contains("boom") && err.contains("while importing module 'm'"), "実際のエラー: {err}");
}

/// ⚠ メタ関数を使うモジュールは**最上位で** import する（関数の中の import の本体は展開されない）。
#[test]
fn a_metafn_module_imported_inside_a_function_is_rejected() {
    let (body, counter, traits) = parse(module_src()).expect("parse");
    let mut imp = import_of(None);
    if let Stmt::Import { body: b, .. } = &mut imp {
        *b = body;
    }
    let tokens = crate::lexer::Lexer::new("fn f() -> None:\n    pass\n", "").tokenize();
    let mut parser = crate::parser::Parser::new(tokens, None);
    parser.set_node_counter(std::rc::Rc::clone(&counter));
    let mut main = parser.parse_program().expect("parse");
    if let Stmt::FnDef { body, .. } = &mut main[0] {
        body.insert(0, imp);
    }
    let err = expand_program(main, counter, traits).expect_err("弾かれること");
    assert!(
        err.contains("module 'm' uses metafunctions, so it must be imported at the top level"),
        "実際のエラー: {err}"
    );
}

/// 展開後の文の中から、名前が `name` のクラス・関数宣言を探す（単相化のテスト用）。
fn find_decl<'a>(out: &'a [Stmt], name: &str) -> Option<(usize, &'a Stmt)> {
    out.iter().enumerate().find(|(_, s)| match s {
        Stmt::ClassDef { name: n, .. } | Stmt::FnDef { name: n, .. } | Stmt::GenDef { name: n, .. } => n == name,
        _ => false,
    })
}

/// ⚠⚠ **テンプレートの具体化に出会った時点で実体を作る**（インライン単相化・参考P・タスク 2-8）。
/// 名前は `Box[int]`、型変数は具体型に置換され、**使う文の直前**に置かれる。
#[test]
fn a_template_class_is_instantiated_before_its_first_use() {
    let out = expand(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    mut items: list[T]\n",
        "\n",
        "let b = Box[int](1, [2])\n",
        "let c = Box[int](3, [4])\n",
    ))
    .expect("expand");
    let (at, decl) = find_decl(&out, "Box[int]").expect("`Box[int]` が作られていない");
    let Stmt::ClassDef { body, template_params, .. } = decl else { panic!("ClassDef を期待") };
    assert!(template_params.is_empty(), "具体化は型パラメータを持たない");
    let fields: Vec<(String, String)> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Field { name, type_ann, .. } => Some((name.clone(), type_ann.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        fields,
        vec![("v".to_string(), "int".to_string()), ("items".to_string(), "list[int]".to_string())],
        "合成型の中の `T` も置換されること"
    );
    let first_use = out.iter().position(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "b")).unwrap();
    assert!(at < first_use, "使う文より前に置かれること");
    let count = out.iter().filter(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Box[int]")).count();
    assert_eq!(count, 1, "同じ型引数の具体化は 1 回だけ（メモ化）");
}

/// ⚠ テンプレート関数も同じ（`ident[str]("s")` → `fn ident[str](a: str) -> str`）。
#[test]
fn a_template_function_is_instantiated() {
    let out = expand(concat!(
        "fn ident[T](let a: T) -> T:\n",
        "    return a\n",
        "\n",
        "print(ident[str](\"s\"))\n",
    ))
    .expect("expand");
    let (_, decl) = find_decl(&out, "ident[str]").expect("`ident[str]` が作られていない");
    let Stmt::FnDef { params, return_type, .. } = decl else { panic!("FnDef を期待") };
    assert_eq!(params[0].type_ann.as_deref(), Some("str"));
    assert_eq!(return_type.as_deref(), Some("str"));
}

/// ⚠⚠ **テンプレートの宣言より前**に書いた具体化（関数の本体の中など）も作る（タスク 2-14）。
/// 宣言に出会った時点で作り、**宣言の直後**に置く。関数の本体が呼ばれるのはもっと後なので、
/// 実行時の意味は変わらない。以前は作らなかったので、その具体化の検査が丸ごと抜けていた。
#[test]
fn an_instantiation_written_before_the_template_is_made_after_it() {
    let out = expand(concat!(
        "fn early() -> int:\n",
        "    return Box[int](1).v\n",
        "\n",
        "class Box[T]:\n",
        "    mut v: T\n",
        "\n",
        "print(early())\n",
    ))
    .expect("expand");
    let (tmpl, _) = find_decl(&out, "Box").expect("テンプレート");
    let (inst, _) = find_decl(&out, "Box[int]").expect("宣言より前の具体化も作られること");
    assert!(tmpl < inst, "テンプレートの宣言の後ろに置かれること");
}

const NAMED_SRC: &str = concat!(
    "trait Named:\n",
    "    fn label(self) -> str:\n",
    "        ...\n",
    "\n",
    "class Cat(Named):\n",
    "    mut n: int\n",
    "    fn label(self) -> str:\n",
    "        return \"cat\"\n",
    "\n",
    "class Holder[T: Named]:\n",
    "    mut v: T\n",
    "\n",
);

/// ⚠⚠ 制約付きのテンプレートは、制約を満たすときに作る（タスク 2-14）。判定は実行時と同じ
/// （型引数がクラスで、基底にその trait がある）。
#[test]
fn a_constrained_template_is_instantiated_when_the_constraint_holds() {
    let out = expand(&format!("{NAMED_SRC}let ok = Holder[Cat](Cat(1))\n")).expect("expand");
    assert!(find_decl(&out, "Holder[Cat]").is_some(), "制約を満たす具体化は作る");
}

/// ⚠⚠ 制約違反・型引数の数違いは**展開時のエラー**（D36・タスク 2-16）。実行時の具体化を
/// 廃止したので、以前のように作らずに実行時の `TemplateError` へ任せる経路はもう無い。文面は実行時と同じ。
#[test]
fn a_constraint_violation_and_a_wrong_arity_are_expansion_errors() {
    let err = expand(&format!("{NAMED_SRC}let bad = Holder[int](1)\n")).expect_err("弾かれること");
    assert!(
        err.contains("TemplateError: type `int` does not satisfy trait `Named` (required for template parameter `T`)"),
        "実際のエラー: {err}"
    );
    let err = expand(&format!("{NAMED_SRC}let bad = Holder[Cat, int](1)\n")).expect_err("弾かれること");
    assert!(err.contains("TemplateError: expected 1 type argument(s), got 2"), "実際のエラー: {err}");
}

/// ⚠ まだ宣言されていないクラスを型引数にした制約付きの具体化（関数の本体の中など）は、
/// そのクラスの宣言に出会った時点で作る（タスク 2-16）。関数が呼ばれるのはもっと後。
#[test]
fn a_constrained_instantiation_waits_for_its_class() {
    let out = expand(concat!(
        "trait Named:\n",
        "    fn label(self) -> str:\n",
        "        ...\n",
        "\n",
        "class Holder[T: Named]:\n",
        "    mut v: T\n",
        "\n",
        "fn make() -> str:\n",
        "    return Holder[Dog](Dog(1)).v.label()\n",
        "\n",
        "class Dog(Named):\n",
        "    mut n: int\n",
        "    fn label(self) -> str:\n",
        "        return \"dog\"\n",
        "\n",
        "print(make())\n",
    ))
    .expect("expand");
    let (dog, _) = find_decl(&out, "Dog").expect("Dog");
    let (inst, _) = find_decl(&out, "Holder[Dog]").expect("クラスの宣言に出会ったら作る");
    assert!(dog < inst, "クラスの宣言の後ろに置く");
}

/// ⚠⚠ `^Box[int]` で具体化のメタ情報を引ける（タスク 2-8）。フィールドの型は置換後のもの。
/// ⚠ 呼び出しの付かない `^Box[int]` は以前、添字（`Box` の `int` 番目）として読まれていた。
#[test]
fn caret_on_a_template_instantiation_sees_the_concrete_declaration() {
    let out = expand(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "\n",
        "exprconst !fn field_type_of(m) -> None:\n",
        "    let t = m.fields[0].type\n",
        "    quote code:\n",
        "        let copy: <! t !> = 0\n",
        "\n",
        "field_type_of(^Box[int])\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, Some(t), _, _) if n == "copy" && t == "int")),
        "`^Box[int].fields[0].type` が `int` になっていない"
    );
}

/// ⚠ 置換は**識別子単位**。`Tag` の中の `T` のような部分一致はしない。
#[test]
fn subst_type_replaces_whole_identifiers_inside_composite_types() {
    let m: std::collections::HashMap<String, String> =
        [("T".to_string(), "int".to_string()), ("K".to_string(), "str".to_string())].into_iter().collect();
    use crate::template_subst::subst_type;
    assert_eq!(subst_type("T", &m), "int");
    assert_eq!(subst_type("list[T]", &m), "list[int]");
    assert_eq!(subst_type("dict[K, T]", &m), "dict[str, int]");
    assert_eq!(subst_type("Option[Box[T]]", &m), "Option[Box[int]]");
    assert_eq!(subst_type("Tag", &m), "Tag");
    assert_eq!(subst_type("function[T]->T", &m), "function[int]->int");
}

/// ⚠⚠ **再帰的な具体化は終わらない**ので、深さの上限で止めて連鎖を見せる（タスク 2-9）。
/// `f[T]` が `f[Box[T]]` を呼ぶと型引数が伸び続ける。展開時の単相化は書かれた具体化を
/// すべて先に作るので、実行時の再帰が途中で止まる書き方でも終わらない。
/// 以前（2-8）は上限まで黙って作り続けて 2 秒以上かかった（実測）。
#[test]
fn a_recursive_template_instantiation_is_reported() {
    let err = expand(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "\n",
        "fn depth[T](let x: T, let n: int) -> int:\n",
        "    if n == 0:\n",
        "        return 0\n",
        "    return 1 + depth[Box[T]](Box[T](x), n - 1)\n",
        "\n",
        "print(depth[int](1, 3))\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("template instantiation does not end")
            && err.contains("'depth[int]' → 'depth[Box[int]]' → 'depth[Box[Box[int]]]'"),
        "実際のエラー: {err}"
    );
}

/// ⚠ 自分自身を**同じ型引数で**使うテンプレート（型レベルの再帰）は止まる。
/// メモ化の印を置換の前に付けているので、`Node[int]` の中の `Node[int]` で潜り直さない。
#[test]
fn a_template_that_uses_itself_with_the_same_arguments_terminates() {
    let out = expand(concat!(
        "class Node[T]:\n",
        "    mut v: T\n",
        "    mut next: Option[Node[T]]\n",
        "\n",
        "    fn prepend(self, let x: T) -> Node[T]:\n",
        "        return Node[T](x, None)\n",
        "\n",
        "let n = Node[int](1, None)\n",
    ))
    .expect("expand");
    let count = out.iter().filter(|s| matches!(s, Stmt::ClassDef { name, .. } if name == "Node[int]")).count();
    assert_eq!(count, 1);
}

/// 展開してから型検査にかけ、エラーの文面（色付けを外したもの）を返す（タスク 2-8 段階 2）。
fn expand_and_check(src: &str) -> Vec<String> {
    let out = expand(src).expect("expand");
    let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
    errors
        .iter()
        .map(|e| {
            let s = e.to_string();
            let mut m = String::new();
            let mut esc = false;
            for c in s.chars() {
                if esc {
                    esc = c != 'm';
                } else if c == '\x1b' {
                    esc = true;
                } else {
                    m.push(c);
                }
            }
            m
        })
        .collect()
}

/// ⚠⚠ **具体化した本体を型検査にかける**（2026-09-26 決定・タスク 2-8 段階 2）。
/// `Box[str]` の `self.v = 0` は誤り。以前は「`T` とは突き合わせない」として通していた。
#[test]
fn a_concrete_type_error_in_an_instantiated_body_is_reported() {
    let errs = expand_and_check(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    fn reset(mut self) -> None:\n",
        "        self.v = 0\n",
        "let a = Box[int](1)\n",
        "let b = Box[str](\"x\")\n",
    ));
    assert!(errs.iter().any(|m| m.contains("field 'v' of class 'Box[str]' is declared 'str' but got 'int'")), "出ていない: {errs:?}");
    assert!(!errs.iter().any(|m| m.contains("class 'Box[int]'")), "出てはいけない: {errs:?}");
}

/// ⚠ 同じ本体でも、型が合う具体化（`Box[int]`）だけなら通る。
#[test]
fn an_instantiation_where_the_body_fits_is_accepted() {
    let errs = expand_and_check(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    fn reset(mut self) -> None:\n",
        "        self.v = 0\n",
        "let a = Box[int](1)\n",
    ));
    assert!(errs.is_empty(), "偽陽性: {errs:?}");
}

/// ⚠⚠ **テンプレートクラスのメソッド呼び出しが丸ごと検査されていなかった**（フェーズ10 の発端）。
/// 受け手のクラスを引く所が `GenericInstance` を見ておらず、検査が飛ばされていた。
#[test]
fn method_calls_on_a_template_instance_are_checked() {
    let errs = expand_and_check(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    fn get(self) -> T:\n",
        "        return self.v\n",
        "    fn set(mut self, let x: T) -> None:\n",
        "        self.v = x\n",
        "    fn tag(mut self, let n: int) -> None:\n",
        "        print(n)\n",
        "mut b = Box[int](1)\n",
        "b.tag(\"wrong\")\n",
        "b.set(\"wrong\")\n",
        "let s: str = b.get()\n",
    ));
    assert!(errs.iter().any(|m| m.contains("argument 0 of 'Box[int].tag' expects 'int' but got 'str'")), "出ていない: {errs:?}");
    assert!(errs.iter().any(|m| m.contains("argument 0 of 'Box[int].set' expects 'int' but got 'str'")), "出ていない: {errs:?}");
    assert!(errs.iter().any(|m| m.contains("'s' is declared 'str' but initialized with 'int'")), "出ていない: {errs:?}");
}

/// ⚠⚠ `gen take[T]` の `T` がスコープに入らず誤って弾かれていた（フェーズ10 10-1・8.5 の退行）。
#[test]
fn a_template_gen_sees_its_type_parameters() {
    let errs = expand_and_check(concat!(
        "gen take[T](let xs: list[T], let n: int) -> T:\n",
        "    mut i = 0\n",
        "    while i < n:\n",
        "        yield xs[i]\n",
        "        i += 1\n",
        "for x in take[int]([1, 2], 2):\n",
        "    print(x)\n",
    ));
    assert!(errs.is_empty(), "偽陽性: {errs:?}");
}

/// ⚠ `mut items: list[T]` の自動 `__init__` が「`list[unknown]` に `list[T]`」と誤って弾かれていた。
/// 具体化では要素の型違いがちゃんと捕まる。
#[test]
fn a_list_field_of_a_type_parameter_is_accepted_and_checked() {
    let errs = expand_and_check(concat!(
        "class Tagged[T]:\n",
        "    mut v: T\n",
        "    mut items: list[T]\n",
        "let good = Tagged[int](1, [2])\n",
        "let bad = Tagged[int](1, [\"x\"])\n",
    ));
    assert!(errs.iter().any(|m| m.contains("expects 'list[int]' but got 'list[str]'")), "出ていない: {errs:?}");
    assert!(!errs.iter().any(|m| m.contains("list[unknown]")), "出てはいけない: {errs:?}");
}

/// ⚠ 型変数に関係しない誤りは、テンプレートの本体と各具体化の**全部で**見つかる。
/// 同じ誤りを具体化の数＋1 回出さないよう、1 件にまとめる（タスク 2-8 段階 2）。
#[test]
fn the_same_error_is_not_repeated_for_every_instantiation() {
    let errs = expand_and_check(concat!(
        "class Labeled[T]:\n",
        "    mut v: T\n",
        "    fn label(self) -> str:\n",
        "        return 1\n",
        "let a = Labeled[int](1)\n",
        "let b = Labeled[str](\"x\")\n",
    ));
    let n = errs.iter().filter(|m| m.contains("declared to return 'str' but returns 'int'")).count();
    assert_eq!(n, 1, "重複している: {errs:?}");
}

/// ⚠⚠ 適合しているテンプレートの具体化を protocol 型に入れると「クラスのインスタンスではない」と
/// **誤って弾いていた**（タスク 2-14）。単相化した具体クラスで適合を見る。適合しないものは理由つきで弾く。
#[test]
fn a_template_instance_is_checked_against_a_protocol_structurally() {
    let src = |ret: &str| {
        format!(
            "protocol Sized:\n    fn size(self) -> int:\n        ...\n\nclass Box[T]:\n    mut v: T\n    fn size(self) -> {ret}:\n        return 1\n\nlet s: Sized = Box[int](1)\n"
        )
    };
    let ok = expand_and_check(&src("int"));
    assert!(ok.is_empty(), "偽陽性: {ok:?}");
    let bad = expand_and_check(&src("str"));
    assert!(
        bad.iter().any(|m| m.contains("type 'Box[int]' does not satisfy protocol 'Sized'")),
        "出ていない: {bad:?}"
    );
}

/// ⚠⚠ テンプレートの本体で `T` の値を具体型の枠へ入れる形（`let y: int = x`・`h(x)`・`return x`・
/// `self.set("x")`）は、`g[int]` / `Box[str]` しか使っていなければ正しい（タスク 2-15）。
/// 以前は期待型の側の `T` しか除外しておらず、値の側の `T` で偽のエラーになっていた。
/// 型引数が合わない具体化は、具体化の本体で捕まる。
#[test]
fn a_type_parameter_on_either_side_is_left_to_the_instantiations() {
    let src = |targ: &str, arg: &str| {
        format!(
            "fn h(let n: int) -> int:\n    return n\n\nfn g[T](let x: T) -> int:\n    let y: int = x\n    print(h(x))\n    return x\n\nclass Box[T]:\n    mut v: T\n    fn set(mut self, let x: T) -> None:\n        self.v = x\n    fn fill(mut self) -> None:\n        self.set(\"x\")\n\nprint(g[{targ}]({arg}))\nmut b = Box[{targ}]({arg})\nb.fill()\n"
        )
    };
    let ok = expand_and_check(&src("int", "1").replace("Box[int](1)", "Box[str](\"a\")"));
    assert!(ok.is_empty(), "偽陽性: {ok:?}");
    let bad = expand_and_check(&src("str", "\"a\"").replace("Box[str](\"a\")", "Box[int](1)"));
    assert!(bad.iter().any(|m| m.contains("'y' is declared 'int' but initialized with 'str'")), "{bad:?}");
    assert!(bad.iter().any(|m| m.contains("'g[str]' is declared to return 'int' but returns 'str'")), "{bad:?}");
    assert!(bad.iter().any(|m| m.contains("argument 0 of 'Box[int].set' expects 'int' but got 'str'")), "{bad:?}");
}

/// ⚠ 関数型の中の型変数も置換する（タスク 2-15）。`adder[int](1)` が `function[T]->T` のまま
/// 外へ出て、`f(2)` が「`T` を期待したのに `int`」という偽のエラーになっていた。
#[test]
fn a_function_type_returned_from_a_template_function_is_substituted() {
    let errs = expand_and_check(concat!(
        "fn adder[T](let base: T) -> function[T]->T:\n",
        "    fn add(let x: T) -> T:\n",
        "        return base + x\n",
        "    return add\n",
        "let f = adder[int](1)\n",
        "print(f(2))\n",
        "print(f(\"s\"))\n",
    ));
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].contains("argument 0 of 'f' expects 'int' but got 'str'"), "{errs:?}");
}

fn template_module_src() -> &'static str {
    concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "    fn set(mut self, let x: T) -> None:\n",
        "        self.v = x\n",
        "\n",
        "let inner = Box[str](\"own\")\n",
    )
}

fn body_of<'a>(out: &'a [Stmt], i: usize) -> &'a [Stmt] {
    match &out[i] {
        Stmt::Import { body, .. } | Stmt::FromImport { body, .. } => body,
        _ => panic!("import を期待"),
    }
}

/// ⚠⚠ `import` したモジュールのテンプレートの具体化（`m.Box[int]` / `from m import Box`）は、
/// **その `import` 文の本体の末尾**に置く（タスク 2-15）。メインに置くと、クラスの本体が
/// モジュールの名前ではなくメインの名前を引いてしまう。以前は作らなかったので検査が丸ごと抜けていた。
#[test]
fn an_imported_template_is_instantiated_inside_the_module_body() {
    for (imp, main) in [
        (import_of(None), "let b = m.Box[int](1)\n"),
        (from_import_of(&["Box"]), "let b = Box[int](1)\n"),
    ] {
        let out = expand_with_module(template_module_src(), &[imp], main).expect("expand");
        assert!(find_decl(&out, "Box[int]").is_none(), "メインには置かない");
        let body = body_of(&out, 0);
        let (at, _) = find_decl(body, "Box[int]").expect("モジュールの本体に置く");
        assert_eq!(at, body.len() - 1, "本体の末尾に置く");
    }
}

/// ⚠ モジュール**自身の中の**具体化も、その本体の中で単相化する（タスク 2-15）。
/// 使う文より前に置く（メインのプログラムと同じ規則）。
#[test]
fn a_module_s_own_instantiation_is_placed_before_its_use() {
    let out = expand_with_module(template_module_src(), &[import_of(None)], "print(1)\n").expect("expand");
    let body = body_of(&out, 0);
    let (at, _) = find_decl(body, "Box[str]").expect("モジュールの中の具体化");
    let use_at = body.iter().position(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "inner")).unwrap();
    assert!(at < use_at, "使う文より前に置く");
}

/// ⚠⚠ `import` 先の具体化の型の誤りも出る（タスク 2-15）。具体化の本体で見つかった誤り（型引数を選んだ側の
/// 誤り）と、テンプレートの本体の誤り（モジュール自身の誤り・known_bugs.md の #21 で import 側にも出す）の
/// 両方が出る。テンプレートの本体でも出る誤りは、具体化の分を落として 1 回だけ出す。
#[test]
fn a_type_error_in_an_imported_instantiation_is_reported() {
    let out = expand_with_module(
        concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "    fn reset(mut self) -> None:\n",
            "        self.v = 0\n",
            "    fn label(self) -> str:\n",
            "        return 1\n",
        ),
        &[import_of(None)],
        "mut b = m.Box[str](\"x\")\nb.reset()\n",
    )
    .expect("expand");
    let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
    let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
    assert!(msgs.iter().any(|m| m.contains("Box[str]") && m.contains("declared")), "出ていない: {msgs:?}");
    // テンプレートの本体の誤り（`label`）はモジュール自身の誤りとして 1 回だけ出す（具体化の同じ誤りは落とす）。
    assert_eq!(msgs.iter().filter(|m| m.contains("label")).count(), 1, "1 回だけ出す: {msgs:?}");
}

/// ⚠⚠ **import したモジュールの本体の誤りは import した側の誤りとしても出る**（known_bugs.md の #21 ①）。
/// 何か所から import しても 1 回だけ。以前は捨てていたので、`str` の変数に `1` が入ったまま動いた。
#[test]
fn a_module_s_own_errors_are_reported_to_the_importer() {
    let out = expand_with_module(
        concat!(
            "let s: str = 1\n",
            "fn never_called() -> None:\n",
            "    print(missing_in_module)\n",
        ),
        &[import_of(None), from_import_of(&["s"])],
        "print(m.s)\n",
    )
    .expect("expand");
    let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
    let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
    // 文面は名前を ANSI で色付けしている（`'s'` の引用符の間に制御文字が入る）ので、文言の部分で数える。
    assert_eq!(msgs.iter().filter(|m| m.contains("is declared")).count(), 1, "{msgs:?}");
    assert_eq!(msgs.iter().filter(|m| m.contains("missing_in_module")).count(), 1, "{msgs:?}");
}

/// ⚠⚠ **モジュールの本体から import した側の大域は見えない**（known_bugs.md の #21 ②）。以前は import した側の
/// 大域の上で検査していたので型検査は通り、実行時に `NameError` になっていた（モジュールの大域は別）。
/// 組み込みの名前（`len`）は見える。
#[test]
fn a_module_body_does_not_see_the_importer_s_globals() {
    let out = expand_with_module_after(
        "let MAIN_G = 1\n",
        concat!(
            "fn read_main() -> int:\n",
            "    return MAIN_G + len(\"ab\")\n",
        ),
        &[import_of(None)],
        "print(m.read_main())\n",
    )
    .expect("expand");
    let (errors, _, _) = crate::type_check::TypeChecker::check_program(&out);
    let msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(msgs[0].contains("MAIN_G") && msgs[0].contains("not defined"), "{msgs:?}");
}

#[test]
fn split_instance_name_is_the_inverse_of_instance_name() {
    use crate::template_subst::{instance_name, split_instance_name};
    for args in [vec!["int"], vec!["Pair[int, str]", "int"], vec!["function[int, str]->int"]] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        let name = instance_name("Box", &args);
        assert_eq!(split_instance_name(&name), Some(("Box", args.clone())));
    }
    assert_eq!(split_instance_name("Box"), None);
}

/// ⚠⚠ **具体化どうしで node-id を共有しない**（設計書 §0.3・タスク 2-13）。
/// 型検査の注釈は node-id で引かれるので、共有すると後に検査した具体化の型が全部に効く
/// （`add_all[int]` の `a + b` が float の加算 `FBIN` にコンパイルされた・実測）。
#[test]
fn instantiations_get_their_own_node_ids() {
    let out = expand(concat!(
        "fn add_all[T](let a: T, let b: T) -> T:\n",
        "    return a + b\n",
        "\n",
        "print(add_all[int](1, 2))\n",
        "print(add_all[float](1.5, 2.5))\n",
    ))
    .expect("expand");
    let add_id = |name: &str| -> u32 {
        let (_, decl) = find_decl(&out, name).expect("宣言が無い");
        let Stmt::FnDef { body, .. } = decl else { panic!("FnDef を期待") };
        let Stmt::Return(Some(Expr::BinOp { node_id, .. }), _) = &body[0] else { panic!("return a + b を期待") };
        *node_id
    };
    let (t, i, f) = (add_id("add_all"), add_id("add_all[int]"), add_id("add_all[float]"));
    assert!(t != i && t != f && i != f, "node-id が衝突している: {t} {i} {f}");
}

/// ⚠⚠ 展開時は**決定的で副作用の無い**ビルトインしか呼べない（D26 / 参考N・タスク 3-7）。
/// 強制していなかった頃は、メタ関数の中で `open` がそのまま通ってファイルを開けた（実測）。
/// エディタが展開を走らせるようになると（D5）、打鍵のたびにファイルを触ることになる。
#[test]
fn forbidden_builtins_are_rejected_while_expanding() {
    for (call, name) in [
        ("open(\"x.txt\", FileOpenMode.read)", "open"),
        ("id(1)", "id"),
        ("getenv(\"PATH\")", "getenv"),
        ("parse_ar(\"let a = 1\")", "parse_ar"),
    ] {
        let src = format!(
            concat!(
                "exprconst !fn sneaky() -> None:\n",
                "    let r = {}\n",
                "    quote code:\n",
                "\n",
                "sneaky()\n",
            ),
            call
        );
        let err = expand(&src).expect_err(name);
        assert!(
            err.contains(&format!("`{name}` cannot be called while expanding")),
            "{name} が通ってしまった: {err}"
        );
    }
}

/// ⚠ 受け手の型で止める（メソッド名で並べると後から足したメソッドが素通りする）。
/// `EventLoop` は大域の単一値なので、生成を止めるだけでは足りない。
#[test]
fn the_event_loop_cannot_be_touched_while_expanding() {
    let err = expand(concat!(
        "exprconst !fn ev() -> None:\n",
        "    EventLoop.run()\n",
        "    quote code:\n",
        "\n",
        "ev()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("the event loop cannot be used while expanding"), "実際のエラー: {err}");
}

/// ⚠ 許可されたビルトインは今まで通り使える（厳しすぎないことの対照）。
#[test]
fn allowed_builtins_still_work_while_expanding() {
    let out = expand(concat!(
        "exprconst !fn count_up() -> None:\n",
        "    let n = len([1, 2, 3]) + int(\"4\")\n",
        "    let name = \"v\" + str(n)\n",
        "    quote code:\n",
        "        let <! name !> = 1\n",
        "\n",
        "count_up()\n",
    ))
    .expect("expand");
    assert!(matches!(&out[0], Stmt::Let(n, _, _, _) if n == "v7"));
}

/// ⚠⚠ **装飾子で関数を包む**（タスク 2-10 の本来の用途）。元の本体を別名で残し、
/// 同名のラッパーを置く。`Code` の `+` と 1 行ずつの反復が無いと書けない。
#[test]
fn a_decorator_can_wrap_a_function_keeping_its_body() {
    let out = expand(concat!(
        "exprconst !fn logged(m) -> None:\n",
        "    let impl_name = m.name + \"_impl\"\n",
        "    mut first = True\n",
        "    mut rest = code:\n",
        "    for line in m.code():\n",
        "        if first:\n",
        "            first = False\n",
        "        else:\n",
        "            rest = rest + line\n",
        "    let head = code:\n",
        "        fn <! impl_name !>(x: int) -> int:\n",
        "    let wrapper = code:\n",
        "        fn <! m.name !>(x: int) -> int:\n",
        "            return <! impl_name !>(x)\n",
        "    quote head + rest + wrapper\n",
        "\n",
        "!logged\n",
        "fn double(x: int) -> int:\n",
        "    let y = x * 2\n",
        "    return y\n",
    ))
    .expect("expand");
    let fns: Vec<(&str, usize)> = out
        .iter()
        .filter_map(|s| match s {
            Stmt::FnDef { name, body, .. } => Some((name.as_str(), body.len())),
            _ => None,
        })
        .collect();
    assert_eq!(
        fns,
        vec![("double_impl", 2), ("double", 1)],
        "元の本体（2 文）が別名で残り、同名のラッパーが置かれること"
    );
}

/// `.indent()` はメソッド形で `+` より強く結合する（§1.2）。`if` の中へ入れられる。
#[test]
fn indent_nests_a_fragment_under_another() {
    let out = expand(concat!(
        "exprconst !fn guarded() -> None:\n",
        "    let body = code:\n",
        "        let inside = 1\n",
        "    let guard = code:\n",
        "        if True:\n",
        "    quote guard + body.indent()\n",
        "\n",
        "guarded()\n",
    ))
    .expect("expand");
    let Stmt::If { .. } = &out[0] else { panic!("`if` が置かれていない") };
}

/// ⚠⚠ **0 未満になる行が生じる `.dedent()` はエラー**（§1.2）。
/// 黙って 0 で止めると、入れ子の関係が崩れた断片が「それらしく」置かれる。
#[test]
fn dedent_below_the_start_is_an_error() {
    let err = expand(concat!(
        "exprconst !fn bad() -> None:\n",
        "    let c = code:\n",
        "        print(1)\n",
        "    quote c.dedent()\n",
        "\n",
        "bad()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("left of its start"), "実際のエラー: {err}");
}

/// ⚠⚠ `.code()` は**本体ごと**元のコードを返す（設計書 §1.5 / タスク 4-7）。
/// 本体が黙って落ちる `.code()` は最悪の失敗形（`stub_gen` を流用しなかった理由）。
#[test]
fn code_returns_the_whole_declaration_including_its_body() {
    let out = expand(concat!(
        "exprconst !fn duplicate(m) -> None:\n",
        "    quote m.code()\n",
        "\n",
        "!duplicate\n",
        "fn triple(x: int) -> int:\n",
        "    let y = x * 3\n",
        "    return y\n",
    ))
    .expect("expand");
    let Stmt::FnDef { name, body, .. } = &out[0] else { panic!("FnDef を期待") };
    assert_eq!(name, "triple");
    assert_eq!(body.len(), 2, "本体の 2 文が残っていること");
}

/// `.declared_at` は宣言が書かれた位置（参考B #7）。
#[test]
fn declared_at_points_at_the_declaration() {
    let out = expand(concat!(
        "\n",
        "\n",
        "fn area(w: int) -> int:\n",   // 3 行目
        "    return w\n",
        "\n",
        "exprconst !fn where_is(m) -> None:\n",
        "    let loc = m.declared_at\n",
        "    let n = \"line_\" + str(loc.line)\n",
        "    quote code:\n",
        "        let <! n !> = 1\n",
        "\n",
        "where_is(^area)\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "line_3")),
        "3 行目を指していない"
    );
}

/// ⚠ 合成された宣言（自動 `__init__` など）には元のコードが無い。**そう言う**。
/// 空の `Code` を返すと「本体が空の関数」と読めてしまう。
#[test]
fn code_of_a_generated_declaration_says_so() {
    let err = expand(concat!(
        "class Point:\n",
        "    mut x: int\n",
        "\n",
        "exprconst !fn show(m) -> None:\n",
        "    let init = m.methods[0]\n",
        "    let c = init.code()\n",
        "    quote code:\n",
        "\n",
        "show(^Point)\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("was generated rather than written in source"), "実際のエラー: {err}");
}

/// 展開時型推論（設計書 D15 / タスク 4-4）。注釈の無い束縛でも `.type` が答えを持つ。
/// ⚠ 新しい推論器は書いていない。型検査器を「ここまでの文」にかけて束縛の型を読むだけ。
#[test]
fn an_unannotated_binding_has_an_inferred_type() {
    let out = expand(concat!(
        "class Point:\n",
        "    mut x: int\n",
        "\n",
        "let origin = Point(0)\n",
        "\n",
        "exprconst !fn same_type(m) -> None:\n",
        "    let t = m.type\n",
        "    quote code:\n",
        "        let <! t !> = 1\n",
        "\n",
        "same_type(^origin)\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "Point")),
        "`origin` の型が `Point` と推論されていない"
    );
}

/// ⚠ 装飾子の対象になった束縛は**まだ展開済みの文に入っていない**。
/// ⇒ 推論の前提に足して推論する（足さないと「その名前は無い」になる）。
#[test]
fn a_decorated_binding_has_an_inferred_type_too() {
    let out = expand(concat!(
        "exprconst !fn typed_copy(m) -> None:\n",
        "    let t = m.type\n",
        "    quote code:\n",
        "        let copy_of: <! t !> = 0\n",
        "\n",
        "!typed_copy\n",
        "let source = 42\n",
    ))
    .expect("expand");
    let Some(Stmt::Let(_, Some(t), _, _)) = out.first() else {
        panic!("注釈付きの `let` が置かれていない")
    };
    assert_eq!(t, "int");
}

/// ⚠⚠ 推論できなければ**エラー**。`unknown` のような文字列を型名として返すと、
/// スプライスされたときに壊れた型注釈が黙って通る。
#[test]
fn an_uninferable_binding_type_is_an_error() {
    let err = expand(concat!(
        "exprconst !fn show(m) -> None:\n",
        "    let t = m.type\n",
        "    quote code:\n",
        "\n",
        "let mystery = unknown_thing\n",
        "show(^mystery)\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("could not be inferred at expansion time"),
        "実際のエラー: {err}"
    );
}

/// ⚠⚠ スプライスした `str` は**識別子として正しくなければならない**（タスク 4-3）。
/// 確かめないと空白入りの変数が黙って作られる（`"a b"` で実際に完走した）。
#[test]
fn a_spliced_str_that_is_not_an_identifier_is_rejected() {
    for bad in ["a b", "list[int]", "if", "1x", ""] {
        let src = format!(
            concat!(
                "exprconst !fn bind(n: str) -> None:\n",
                "    quote code:\n",
                "        let <! n !> = 1\n",
                "\n",
                "bind({:?})\n",
            ),
            bad
        );
        let err = expand(&src).expect_err(bad);
        assert!(
            err.contains("is not a single identifier"),
            "{bad:?} が通ってしまった: {err}"
        );
    }
}

/// ⚠ 正しい識別子は今まで通り通る（上の検査が厳しすぎないことの対照）。
#[test]
fn a_spliced_str_that_is_an_identifier_is_accepted() {
    for good in ["answer", "_private", "Point", "x1"] {
        let src = format!(
            concat!(
                "exprconst !fn bind(n: str) -> None:\n",
                "    quote code:\n",
                "        let <! n !> = 1\n",
                "\n",
                "bind({:?})\n",
            ),
            good
        );
        let out = expand(&src).unwrap_or_else(|e| panic!("{good:?} が弾かれた: {e}"));
        assert!(
            matches!(&out[0], Stmt::Let(n, _, _, _) if n == good),
            "{good:?} で束縛されていない"
        );
    }
}

/// ⚠⚠ 置いたコードがさらにメタ関数を呼ぶ形では、失敗した地点だけ見ても
/// **なぜそこにその呼び出しがあるのか**が追えない（タスク 3-3）。
/// ⇒ 出所の列（どのメタ関数がどこから置いたか）を添える。
#[test]
fn a_nested_expansion_reports_where_it_came_from() {
    let err = expand(concat!(
        "exprconst !fn inner() -> None:\n",
        "    compile_error(\"inner refuses\")\n",
        "\n",
        "exprconst !fn outer() -> None:\n",
        "    quote code:\n",
        "        inner()\n",
        "\n",
        "outer()\n",
    ))
    .expect_err("止まること");
    assert!(err.contains("inner refuses"), "実際のエラー: {err}");
    assert!(
        err.contains("while expanding 'inner'"),
        "内側の出所が無い: {err}"
    );
    assert!(
        err.contains("while expanding 'outer'"),
        "外側の出所が無い —— 「なぜそこに inner() があるのか」が追えない: {err}"
    );
}

/// `compile_error("...")` は**利用者が意図して展開を止める**道具（タスク 3-2）。
/// ⚠ 出るのは**書いた文面だけ**。traceback を被せると肝心の一文が埋もれる。
#[test]
fn compile_error_reports_only_the_authors_message() {
    let err = expand(concat!(
        "exprconst !fn need_positive(n: int) -> None:\n",
        "    if n <= 0:\n",
        "        compile_error(\"need_positive requires a positive literal\")\n",
        "    quote code:\n",
        "        print(1)\n",
        "\n",
        "need_positive(0)\n",
    ))
    .expect_err("止まること");
    assert!(
        err.contains("need_positive requires a positive literal"),
        "実際のエラー: {err}"
    );
    assert!(!err.contains("Traceback"), "traceback が被さっている: {err}");
    assert!(!err.contains("MetaAbort"), "内部の例外名が漏れている: {err}");
}

/// ⚠ 条件を満たすときは**普通に置く**（`compile_error` が常に止めてしまわないこと）。
#[test]
fn compile_error_does_not_fire_when_the_check_passes() {
    let out = expand(concat!(
        "exprconst !fn need_positive(n: int) -> None:\n",
        "    if n <= 0:\n",
        "        compile_error(\"nope\")\n",
        "    quote code:\n",
        "        let accepted = 1\n",
        "\n",
        "need_positive(5)\n",
    ))
    .expect("expand");
    assert!(matches!(&out[0], Stmt::Let(n, _, _, _) if n == "accepted"));
}

/// ⚠⚠ 同じメタ関数を 2 回呼ぶと、置いたコードの局所名がぶつかる（タスク 3-1）。
/// `gensym` が毎回違う名前を返すことで初めて「置ける道具」になる。
#[test]
fn gensym_keeps_placed_local_names_apart() {
    let out = expand(concat!(
        "exprconst !fn twice() -> None:\n",
        "    quote code:\n",
        "        let <! gensym(\"tmp\") !> = 1\n",
        "\n",
        "twice()\n",
        "twice()\n",
    ))
    .expect("expand");
    let names: Vec<&str> = out
        .iter()
        .filter_map(|s| match s {
            Stmt::Let(n, _, _, _) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 2, "2 本置かれること");
    assert_ne!(names[0], names[1], "名前が違うこと: {names:?}");
    assert!(names.iter().all(|n| n.starts_with("tmp__")), "接頭辞が付くこと: {names:?}");
}

/// ⚠ `gensym` を使わなければ**ぶつかる**（上のテストが何を守っているかの対照）。
/// ⚠ ぶつかり方は**静的型検査の二重宣言**なので、展開そのものは通る。
#[test]
fn placing_the_same_local_name_twice_collides() {
    let out = expand(concat!(
        "exprconst !fn twice() -> None:\n",
        "    quote code:\n",
        "        let tmp = 1\n",
        "\n",
        "twice()\n",
        "twice()\n",
    ))
    .expect("展開自体は通る");
    let names: Vec<&str> = out
        .iter()
        .filter_map(|s| match s {
            Stmt::Let(n, _, _, _) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["tmp", "tmp"], "同じ名前が 2 本置かれる（この後 型検査が弾く）");
}

/// `const` は**展開時にも値が確定している**ので読める（設計書 §1.7 / タスク 3-6）。
/// ⚠ これが無いと「展開時に分岐する」用途がほぼ書けない（仮引数しか見られない）。
#[test]
fn a_const_is_readable_while_expanding() {
    let out = expand(concat!(
        "const N = 3\n",
        "\n",
        "exprconst !fn pick() -> None:\n",
        "    if N > 1:\n",
        "        quote code:\n",
        "            let big = 1\n",
        "    quote code:\n",
        "        let small = 1\n",
        "\n",
        "pick()\n",
    ))
    .expect("expand");
    let names: Vec<&str> = out
        .iter()
        .filter_map(|s| match s {
            Stmt::Let(n, _, _, _) | Stmt::Const(n, _, _, _) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["N", "big"], "`const` を見て分岐し、`const` 自体は AST に残る");
}

/// ⚠ 初期化子が実行時の値に依存する `const` は展開時に評価できない。
/// その `const` が展開時に使えないだけで**プログラムとしては正しい**ので、
/// 展開を止めず、読まれたときに 3-0 の診断を出す。
#[test]
fn a_const_that_needs_run_time_values_is_not_available_while_expanding() {
    let err = expand(concat!(
        "fn at_run_time() -> int:\n",
        "    return 7\n",
        "\n",
        "const LATE = at_run_time()\n",
        "\n",
        "exprconst !fn pick() -> None:\n",
        "    if LATE > 1:\n",
        "        quote code:\n",
        "            let big = 1\n",
        "    quote code:\n",
        "\n",
        "pick()\n",
    ))
    .expect_err("読めないこと");
    assert!(err.contains("is declared at run time"), "実際のエラー: {err}");
}

/// `^名前` が**自分より前の宣言**からメタ情報を作ること（設計書 §1.5 / タスク 4-0）。
#[test]
fn caret_builds_meta_info_from_an_earlier_declaration() {
    let out = expand(concat!(
        "class Point:\n",
        "    mut x: int\n",
        "\n",
        "exprconst !fn describe(m) -> None:\n",
        "    quote code:\n",
        "        let <! m.name !> = 1\n",
        "\n",
        "describe(^Point)\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "Point")),
        "`.name` が射影されていない: {:?}",
        out.iter().map(crate::interpreter::tw_stats::stmt_kind_of).collect::<Vec<_>>()
    );
}

/// クラスの射影（参考B #2 #4 #5・タスク 4-1）。
/// ⚠⚠ フィールド名は `decl_names` では取れない（あちらはスコープの名前を答える
/// モジュールで、クラスのフィールドは意図的に報告しない）。混ぜると `.name` が
/// **空文字列になる**（実測）。⇒ `MetaValue::name_of` を使う。
#[test]
fn class_projections_read_the_declaration() {
    let out = expand(concat!(
        "class Point:\n",
        "    mut x: int\n",
        "    let label: str\n",
        "\n",
        "    fn greet(self) -> str:\n",
        "        return \"hi\"\n",
        "\n",
        "exprconst !fn report(m) -> None:\n",
        "    let first = m.fields[0].name\n",
        "    let ok = m.has_method(\"greet\")\n",
        "    if ok:\n",
        "        quote code:\n",
        "            let <! first !> = 1\n",
        "    quote code:\n",
        "\n",
        "report(^Point)\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "x")),
        "`.fields[0].name` と `.has_method` が効いていない"
    );
}

/// 関数の射影（参考B #1）。⚠ 仮引数の記録は name / type / mutable / has_default / variadic。
#[test]
fn function_projections_read_the_signature() {
    let out = expand(concat!(
        "fn greet(mut who: str, times: int = 2) -> str:\n",
        "    return who\n",
        "\n",
        "exprconst !fn report(m) -> None:\n",
        "    let n = m.params[0].name + \"_\" + m.return_type.name\n",
        "    quote code:\n",
        "        let <! n !> = 1\n",
        "\n",
        "report(^greet)\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "who_str")),
        "仮引数名と戻り値型が取れていない"
    );
}

/// ⚠⚠ **型注釈にしか現れない具体化も作る**（タスク 10-4）。`fn take(let b: Box[int])` の
/// `Box[int]` は、以前は作られず、型検査がテンプレートの宣言を型引数で置き換えて読んでいた。
#[test]
fn an_instantiation_written_only_in_an_annotation_is_made() {
    let out = expand(concat!(
        "class Box[T]:\n",
        "    mut v: T\n",
        "\n",
        "fn take(let b: Box[int]) -> int:\n",
        "    return b.v\n",
    ))
    .expect("expand");
    assert!(find_decl(&out, "Box[int]").is_some(), "注釈の具体化が作られていない");
}

/// ⚠ 注釈を集める走査は `import` の本体（モジュールの中身）へ降りない（タスク 10-4）。
/// 降りると、モジュールのテンプレートの宣言の中の `Box[V]`（型変数のまま）を具体化してしまった（実測）。
#[test]
fn annotations_inside_an_imported_template_are_not_instantiated() {
    let out = expand_with_module(
        concat!(
            "class Box[T]:\n",
            "    mut v: T\n",
            "\n",
            "class Pair[K, V]:\n",
            "    mut k: K\n",
            "    mut v: V\n",
            "    fn boxed(self) -> Box[V]:\n",
            "        return Box[V](self.v)\n",
        ),
        &[import_of(None)],
        "let p = m.Pair[str, int](\"k\", 1)\n",
    )
    .expect("expand");
    let body = body_of(&out, 0);
    assert!(find_decl(body, "Box[int]").is_some(), "具体化の中の Box[int] は作る");
    let leaked: Vec<&str> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::ClassDef { name, .. } if name.contains("[V]") || name.contains("[K") => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert!(leaked.is_empty(), "型変数のままの具体化: {leaked:?}");
}

/// ⚠⚠ **型は型の値で返る**（D23・タスク 4-2・2026-09-27）。`.type` / 仮引数の `type` /
/// `.return_type` は `int` と等しく、合成（`list[t]` / `dict[k, v]`）も差し込みもそのまま書ける。
#[test]
fn type_projections_return_type_values_that_compose_and_splice() {
    let out = expand(concat!(
        "class Point:\n",
        "    mut x: int\n",
        "    mut names: dict[str, int]\n",
        "\n",
        "exprconst !fn declare(m) -> None:\n",
        "    let t = m.fields[0].type\n",
        "    if t != int:\n",
        "        compile_error(\"not a type value\")\n",
        "    if dict[str, t] != m.fields[1].type:\n",
        "        compile_error(\"a composed type differs from the same annotation\")\n",
        "    quote code:\n",
        "        let xs: <! list[t] !> = []\n",
        "        let d: <! dict[str, t] !> = {}\n",
        "        let <! t.name + \"_named\" !> = 1\n",
        "\n",
        "declare(^Point)\n",
    ))
    .expect("expand");
    let ann = |name: &str| {
        out.iter().find_map(|s| match s {
            Stmt::Let(n, t, _, _) if n == name => t.clone(),
            _ => None,
        })
    };
    assert_eq!(ann("xs").as_deref(), Some("list[int]"));
    // ⚠ 綴りは型注釈と同じ（パーサの正規の綴り・`canonical_type`）。
    assert_eq!(ann("d").as_deref(), Some("dict[str,int]"));
    assert!(out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "int_named")));
}

/// ⚠ 型引数は型に限る。型でない値で合成しようとしたら展開時のエラー（壊れた型を黙って作らない）。
#[test]
fn composing_a_type_with_a_non_type_is_an_error() {
    let err = expand(concat!(
        "exprconst !fn bad() -> None:\n",
        "    let t = list[3]\n",
        "    quote code:\n",
        "        let xs: <! t !> = []\n",
        "\n",
        "bad()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("the type arguments of `list[...]` must be types"), "{err}");
}

/// ⚠⚠ 種別と噛み合わない射影は**エラー**（タスク 4-1）。空のリストを返すと
/// 「フィールドが 0 個」と読めてしまい、取り違えが通ってしまう。
#[test]
fn a_projection_that_does_not_fit_the_kind_is_an_error() {
    let err = expand(concat!(
        "fn greet() -> str:\n",
        "    return \"hi\"\n",
        "\n",
        "exprconst !fn wrong(m) -> None:\n",
        "    let f = m.fields\n",
        "    quote code:\n",
        "\n",
        "wrong(^greet)\n",
    ))
    .expect_err("弾かれること");
    // ⚠ **どの射影か**まで言う（タスク 3-9）。以前は「this projection」とだけ出て、
    //   メタ関数の中に射影が幾つもあると、どれが間違いか分からなかった。
    assert!(
        err.contains("`.fields` needs a class or trait, but 'greet' is a meta_function"),
        "実際のエラー: {err}"
    );
}

/// ⚠ 展開は前方のみ（§1.7）。**後ろの宣言には `^` を当てられない**。
/// ⚠ 「まだ宣言されていない」と「メタ情報を取れない種類」を言い分ける。
#[test]
fn caret_on_a_later_declaration_says_it_is_not_visible_yet() {
    let err = expand(concat!(
        "exprconst !fn d(m) -> None:\n",
        "    quote code:\n",
        "\n",
        "d(^Later)\n",
        "\n",
        "class Later:\n",
        "    mut a: int\n",
    ))
    .expect_err("見えないこと");
    assert!(
        err.contains("refers to nothing declared before this point"),
        "実際のエラー: {err}"
    );
}

/// ⚠ 綴り間違いの射影は**黙って `None` を返さない**。通してしまうと気づかれない。
#[test]
fn an_unknown_projection_is_an_error() {
    let err = expand(concat!(
        "class C:\n",
        "    mut a: int\n",
        "\n",
        "exprconst !fn d(m) -> None:\n",
        "    let x = m.typo\n",
        "    quote code:\n",
        "\n",
        "d(^C)\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("has no projection"), "実際のエラー: {err}");
}

/// 装飾子の第一引数は**メタ情報**（タスク 4-0）。`parse_ar` の `Namespace` 木ではない。
#[test]
fn a_decorator_receives_meta_information() {
    let out = expand(concat!(
        "exprconst !fn rename(m) -> None:\n",
        "    quote code:\n",
        "        let <! m.kind !> = 1\n",
        "\n",
        "!rename\n",
        "class Widget:\n",
        "    mut w: int\n",
    ))
    .expect("expand");
    assert!(
        out.iter().any(|s| matches!(s, Stmt::Let(n, _, _, _) if n == "meta_class")),
        "装飾子が受け取ったのはメタ情報ではない"
    );
}

/// ⚠⚠ 実行時の値を読もうとしたら**そう言う**（設計書 §1.8 / タスク 3-0）。
/// 素の `NameError` のままだと「定義したのに無いと言われた」と読めてしまう。
#[test]
fn reading_a_run_time_value_says_so() {
    let err = expand(concat!(
        "let runtime_v = 5\n",
        "\n",
        "exprconst !fn choose() -> None:\n",
        "    if runtime_v > 3:\n",
        "        quote code:\n",
        "            print(1)\n",
        "    quote code:\n",
        "\n",
        "choose()\n",
    ))
    .expect_err("弾かれること");
    assert!(
        err.contains("is declared at run time"),
        "実際のエラー: {err}"
    );
}

/// ⚠ 実行時の宣言**ではない**名前は言い直さない（元の `NameError` をそのまま出す）。
/// 誤検知で正しい診断を隠さないこと。
#[test]
fn an_unknown_name_keeps_its_original_error() {
    let err = expand(concat!(
        "exprconst !fn choose() -> None:\n",
        "    if never_declared > 3:\n",
        "        quote code:\n",
        "            print(1)\n",
        "    quote code:\n",
        "\n",
        "choose()\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("NameError"), "実際のエラー: {err}");
    assert!(!err.contains("is declared at run time"), "実際のエラー: {err}");
}

/// ⚠⚠ メタ関数が無限再帰したときの診断が**読める形で外へ出る**こと（タスク 2-6）。
/// インタプリタは例外を**番兵文字列**で返すので、そのまま出すと
/// `MetaError: while expanding 'go':  __raise__` という何も分からない診断になる（実測）。
/// ⚠ **大きめのスタックを持つスレッドで走らせる。** インタプリタの再帰上限は 1000 段で、
/// debug ビルドの 1 フレームは release より厚い。既定のテストスレッド（2 MiB）だと
/// **上限が発火する前に Rust 側のスタックが溢れる**（実測: STATUS_STACK_OVERFLOW）。
/// ⚠ これは 2-6 が持ち込んだ性質ではなく、debug ビルドのインタプリタ全体の性質。
#[test]
fn a_recursive_metafn_reports_a_readable_error() {
    let handle = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            expand(concat!(
                "exprconst fn rec(n: int) -> Code:\n",
                "    return rec(n + 1)\n",
                "\n",
                "exprconst !fn go() -> None:\n",
                "    quote rec(0)\n",
                "\n",
                "go()\n",
            ))
            // ⚠ `Vec<Stmt>` は `Rc` を含むのでスレッドを跨げない。見たいのはエラー文字列だけ。
            .map(|_| ())
        })
        .expect("spawn");
    let err = handle.join().expect("join").expect_err("止まること");
    assert!(err.contains("RecursionError"), "実際のエラー: {err}");
    assert!(!err.contains("__raise__"), "番兵がそのまま漏れている: {err}");
}

/// ⚠ 自分自身を置き続けるメタ関数は止まらない。⇒ 歩数上限で切る（本番の診断は 2-6）。
#[test]
fn a_self_placing_metafn_is_stopped_by_the_step_budget() {
    let err = expand(
        "exprconst !fn loop_forever() -> None:\n    quote code:\n        loop_forever()\n\nloop_forever()\n",
    )
    .expect_err("止まること");
    assert!(err.contains("did not finish within"), "実際のエラー: {err}");
}

/// `!装飾子` が対象の宣言を置き換える（設計書 §1.6）。
#[test]
fn a_decorator_replaces_its_target() {
    let out = expand(
        "exprconst !fn swap(target) -> None:\n    quote code:\n        let swapped = 1\n\n!swap\nlet original = 0\n",
    )
    .expect("expand");
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Stmt::Let(name, _, _, _) if name == "swapped"));
}

/// ⚠ 自分より**前**に宣言されたメタ関数しか見えない（§1.7 の逐次展開）。
/// ⚠⚠ 後ろにあるものを呼んだら**展開時のエラー**（前方参照のみ・2026-09-25 確定・タスク 2-11）。
/// 以前は展開されずに残り、実行時の `NameError` になっていた。
#[test]
fn a_metafn_declared_later_is_not_visible() {
    let err = expand(
        "later()\nexprconst !fn later() -> None:\n    quote code:\n        let x = 1\n",
    )
    .expect_err("弾かれること");
    assert!(err.contains("is called before it is declared"), "実際のエラー: {err}");
}

/// ⚠⚠ 置いたコードの node-id は**本体と衝突してはならない**（設計書 §0.3 / タスク 2-1）。
/// 衝突すると型検査の注釈が**別のノードのもの**になる（per-module 採番で FFI 境界検査の
/// 誤検知が実際に再現した）。⇒ 展開器は本体と同じカウンタから採番する。
#[test]
fn placed_nodes_do_not_reuse_node_ids() {
    let src = concat!(
        "exprconst !fn place_it() -> None:\n",
        "    quote code:\n",
        "        let placed = other\n",
        "\n",
        "let other = 1\n",
        "let a = other\n",
        "let b = other\n",
        "let c = other\n",
        "place_it()\n",
    );
    let (stmts, counter, traits) = parse(src).expect("parse");
    // 本体をパースし終えた時点の採番位置。置いたコードはこれより**後ろ**から採番される。
    let before = counter.get();
    assert!(before > 0, "本体が node-id を採番していること");
    let out = expand_program(stmts, std::rc::Rc::clone(&counter), traits).expect("expand");

    let mut ids: Vec<u32> = Vec::new();
    collect_node_ids(&out, &mut ids);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "node-id が重複している: {ids:?}");
    assert!(!ids.contains(&0), "未採番（0）が混ざっている: {ids:?}");
    // ⚠⚠ **これが本命の検査**。カウンタを共有していなければ置いたコードは 1 から
    //   採番され、`before` を超える id は 1 つも出ない（＝ここで落ちる）。
    assert!(
        ids.iter().any(|id| *id > before),
        "置いたコードが本体と同じカウンタから採番されていない (before={before}, ids={ids:?})"
    );
}

/// テスト用: 文の列に現れる `Expr::Ident` の node-id を集める。
fn collect_node_ids(stmts: &[Stmt], out: &mut Vec<u32>) {
    for st in stmts {
        crate::stmt_walk::each_subpart(st, &mut |part| {
            if let crate::stmt_walk::StmtPart::Expr(e) = part {
                collect_expr_ids(e, out);
            }
        });

    }
}

fn collect_expr_ids(e: &Expr, out: &mut Vec<u32>) {
    if let Expr::Ident { node_id, .. } = e {
        out.push(*node_id);
    }
    crate::expr_walk::each_subpart(e, &mut |sub| {
        if let crate::expr_walk::SubPart::Plain(x) | crate::expr_walk::SubPart::Control(x) = sub
        {
            collect_expr_ids(x, out);
        }
    });
}

/// ⚠⚠ **スプライスで差し込んだトークンと、組み直した `Indent`/`Newline` は**
/// **元の位置を持たない**（値やブロック構造には位置が無い）。⇒ 行の位置を代わりに使う
/// （タスク 2-3）。位置不明のまま出すと「どこで間違えたのか分からない」診断になる。
///
/// ⚠ 地の文のトークンは**もともと自分の `Span` を持っている**ので、そちらを見る検査は
/// 2-3 の有無に関わらず通ってしまう。⇒ ここでは**差し込んだトークン**を直接見る。
#[test]
fn spliced_tokens_carry_the_line_position() {
    let src = concat!(
        "exprconst fn f(n) -> Code:\n",      // 1 行目
        "    let a = code:\n",               // 2 行目
        "        let <! n !> = 1\n",         // 3 行目 ← ここを指してほしい
        "    return a\n",
    );
    let (stmts, _, _) = parse(src).expect("parse");
    let Stmt::MetaFnDef { body, .. } = &stmts[0] else { panic!("MetaFnDef を期待") };
    let Stmt::Let(_, _, Expr::CodeBlock(lines), _) = &body[0] else { panic!("CodeBlock を期待") };
    assert_eq!(lines[0].span.line, 3, "行の位置が記録されていること");

    let filled = fill_splices(lines, &[Value::Str("chosen".into())]).expect("fill");
    let spliced = filled[0]
        .pieces
        .iter()
        .filter_map(|p| match p {
            CodePiece::Token(t) => Some(t),
            CodePiece::Splice(_) => None,
        })
        .find(|t| matches!(&t.token, crate::token::Token::Ident(n) if n == "chosen"))
        .expect("差し込んだ識別子があること");
    assert_eq!(spliced.span.line, 3, "差し込んだトークンが行の位置を持つこと");
}

/// 置いたコードのノードが元の位置を保つこと（地の文のトークン由来）。
#[test]
fn placed_nodes_point_back_at_the_code_block_line() {
    let src = concat!(
        "exprconst !fn place_it() -> None:\n",   // 1 行目
        "    quote code:\n",                     // 2 行目
        "        missing_name()\n",              // 3 行目 ← ここを指してほしい
        "\n",
        "place_it()\n",                          // 5 行目
    );
    let out = expand(src).expect("expand");
    assert_eq!(out.len(), 1);
    let Stmt::Expr(Expr::Call { span, .. }) = &out[0] else { panic!("Call を期待") };
    assert_eq!(span.line, 3, "`code:` の中身が書かれていた行を指すこと");
}

/// クラス本体のメンバー装飾子もその場で展開される。
/// ⚠ ここを歩かないと、クラスのメンバーに付けた装飾子が**黙って無視される**。
#[test]
fn decorators_inside_a_class_body_are_expanded() {
    let out = expand(
        "exprconst !fn tag(target) -> None:\n    quote code:\n        mut tagged: int\n\nclass C:\n    !tag\n    mut original: int\n",
    )
    .expect("expand");
    let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
    let fields: Vec<&str> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Field { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec!["tagged"], "装飾された宣言が置き換わっていること");
}

/// ⚠⚠ クラス本体へ置けるのは**メンバー宣言だけ**（タスク 2-5）。
/// `pass` を素通しすると「装飾子が `pass` を置いてメンバーが**黙って消える**」形になる。
/// メンバーを消したいなら空の `Code` を `quote` する（§1.2 の B-2）。
#[test]
fn placing_pass_in_a_class_body_is_rejected() {
    let err = expand(concat!(
        "exprconst !fn passy(t) -> None:\n",
        "    quote code:\n",
        "        pass\n",
        "\n",
        "class C:\n",
        "    mut a: int\n",
        "    !passy\n",
        "    mut b: int\n",
    ))
    .expect_err("弾かれること");
    assert!(err.contains("cannot be placed in a class or trait body"), "実際のエラー: {err}");
}

/// 空の `Code` を置けばメンバーは**意図して消える**（上の `pass` との対）。
#[test]
fn quoting_an_empty_code_removes_the_decorated_member() {
    let out = expand(concat!(
        "exprconst !fn drop_it(t) -> None:\n",
        "    let empty = code:\n",
        "    quote empty\n",
        "\n",
        "class C:\n",
        "    mut a: int\n",
        "    !drop_it\n",
        "    mut b: int\n",
    ))
    .expect("expand");
    let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
    let fields: Vec<&str> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Field { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(fields, vec!["a"], "装飾されたメンバーだけが消えること");
}

/// ⚠⚠ **自動 `__init__` は展開で増えたフィールドを見ていなければならない**（タスク 2-4）。
/// パース時に生成していた頃は、装飾子が足したフィールドが引数に入らず
/// 「そのフィールドを渡せないクラス」ができていた（実測）。
#[test]
fn the_auto_init_sees_fields_added_by_a_decorator() {
    let out = expand(concat!(
        "exprconst !fn add_counter(target) -> None:\n",
        "    quote code:\n",
        "        mut hits: int\n",
        "\n",
        "class Page:\n",
        "    mut title: str\n",
        "    !add_counter\n",
        "    mut placeholder: int\n",
    ))
    .expect("expand");
    let Stmt::ClassDef { body, .. } = &out[0] else { panic!("ClassDef を期待") };
    let init = body
        .iter()
        .find_map(|s| match s {
            Stmt::FnDef { name, params, .. } if name == "__init__" => Some(params),
            _ => None,
        })
        .expect("自動 `__init__` が生成されていること");
    let names: Vec<&str> = init.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["self", "title", "hits"],
        "装飾子が足した `hits` が引数に入り、置き換えられた `placeholder` は消えていること"
    );
}

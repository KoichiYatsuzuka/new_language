// tests/mod.rs — インタープリタ単体テストのモジュール束ね。
// 共通テストヘルパー(run/eval_expr/run_get/run_exc/assert_* 等)を定義し、機能別サブモジュールを宣言する。

use super::*;
use crate::ast::Stmt;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// テストソースを**本番と同じ配線**でパースし、実行可能なインタプリタを作る（#36）。
///
/// `run_program`（main.rs）と同じ 4 点を揃える。1 つでも欠けると、正しいコードでも
/// `VmForceError` になる（`--vm=on` は「解決情報が揃っている」前提の経路）:
///
/// 1. `resolve_program` … ローカル slot・グローバル参照の解決（Phase R）
/// 2. `check_program` … AST 型解決層の注釈（#16）。`mustbe`/`=>` の検査指示はこれが無いと
///    出ないので、**注釈を渡さないと `Expr::MustBe`/`Cast` を含む文が丸ごと bail する**
/// 3. `set_toplevel_globals` … 最上位 Chunk が「この名前は `scopes[0]`」と判断する集合
///
/// ⚠ #33 で `--vm` とツリーウォークは削除された（実行経路は VM 一本）ので、
/// モードの設定は不要になった。**解決情報の供給だけが本番との差**になる。
///
/// ⚠ **型エラーは無視する**。テストには静的検査が弾く形を意図的に実行するものがあり、
/// ここで弾くと検査対象が変わってしまう。欲しいのは注釈だけ。
///
/// ⚠ テストが**本番と同じ経路**（バイトコード VM）を検査することがこの関数の目的。
/// 以前は既定の `Off`＝ツリーウォークで走っており、**本番と違う実装をテストしていた**（#36）。
fn prepare(src: &str) -> Result<(Vec<Stmt>, Interpreter), String> {
    prepare_at(src, None)
}

/// [`prepare`] の、構文解析の起点（エントリのディレクトリ）を指定する版。
/// import 先を探す起点になる（`import[py-int] m` の `m.py` を構文解析の時点で見つけるため）。
fn prepare_at(src: &str, dir: Option<std::path::PathBuf>) -> Result<(Vec<Stmt>, Interpreter), String> {
    let tokens = Lexer::new(src, "").tokenize();
    let mut parser = Parser::new(tokens, dir);
    let stmts = parser.parse_program()?;
    // ⚠ 展開も本番と同じく先に済ませる（D36・タスク 2-16）。テンプレートの具体化は展開時に作り、
    //   実行時には作らないので、これが無いとテンプレートを使うテストが全部落ちる。
    let mut stmts =
        crate::meta_expand::expand_program(stmts, parser.node_counter(), parser.known_traits())?;
    // ⚠ 配線は 1 箇所（#88）。⚠ この入口は**型エラーを無視する**（下の doc の理由）。
    let (_errors, _warnings, annotations) =
        super::resolver::resolve_and_annotate(&mut stmts);
    let mut interp = Interpreter::new();
    interp.wire_resolution(
        annotations,
        super::resolver::toplevel_declared_globals(&stmts),
        crate::interpreter::GlobalsMode::Replace,
    );
    Ok((stmts, interp))
}

/// 静的型検査だけを走らせてエラー一覧を返す（#36）。
///
/// ⚠ **ブロックスコープの再宣言のように「本番では静的検査が捕まえる」規則**は、
/// `run` では検査できない（`prepare` は注釈を得るために型エラーを無視するし、
/// VM の最上位 Chunk は内側 `let` を slot 宣言に落とすので実行時の重複検査を通らない）。
/// そういう規則はこちらで固定する。
fn static_errors(src: &str) -> Vec<crate::type_check::StaticTypeError> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().expect("parse");
    crate::type_check::TypeChecker::check(&stmts)
}

/// テストソースを字句解析・構文解析・実行する。エラーがあれば `Err` を返す。
fn run(src: &str) -> Result<(), String> {
    let (stmts, mut interp) = prepare(src)?;
    for stmt in &stmts {
        let _ = interp.exec(stmt)?;
    }
    Ok(())
}

/// 単一の式文を評価して `Value` を返すテストヘルパー。
///
/// ⚠ **ここはバイトコード経路を通らない**（#36）。`interp.eval()` を直接呼ぶ
/// （VM も一部の式評価をこの関数へ委譲する）ので、VM の配線（`resolve_program` 等）は不要。
/// ⚠ 裏を返すと**このヘルパーを使うテストは VM を検査していない**。
fn eval_expr(src: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    interp
        .eval(match &stmts[0] {
            Stmt::Expr(e) => e,
            _ => panic!("not an expr"),
        })
        .unwrap()
}

/// テストソースを**本番と同じ配線**で実行し、実行後のインタプリタを返す（#36）。
/// 複数の変数を読む／エラー後の状態を見るテスト用（`run_get` は 1 変数だけ）。
fn run_interp(src: &str) -> Interpreter {
    let (stmts, mut interp) = prepare(src).unwrap();
    for stmt in &stmts {
        interp.exec(stmt).unwrap();
    }
    interp
}

/// テストソースを**本番と同じ配線**で実行し、最初に返った内部エラー文字列を返す（#36）。
/// エラーが出なければ空文字列。
fn run_err_msg(src: &str) -> String {
    let (stmts, mut interp) = prepare(src).unwrap();
    for stmt in &stmts {
        if let Err(e) = interp.exec(stmt) {
            return e;
        }
    }
    String::new()
}

/// テストソースを実行して変数 `var` の値を返すテストヘルパー。
fn run_get(src: &str, var: &str) -> Value {
    let (stmts, mut interp) = prepare(src).unwrap();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// py-int テスト用: examples/ ディレクトリを Python 検索パスに追加して実行する
/// ⚠ `native` 限定（評価コアビルドでは `py_interop` がスタブなので成立しない・#6）。
///
/// ⚠ 構文解析も `examples/interop/test_modules` を起点にする。未解決の import はエラーなので
///   （`editor_import_resolution_plan.md` 1-2）、型の出所（`py_calculator.py`）は構文解析の時点で
///   見つからなければならない。以前は構文解析が見つけられず、黙って型なしで通っていた。
#[cfg(feature = "native")]
fn run_py_get(src: &str, var: &str) -> Value {
    let (stmts, mut interp) =
        prepare_at(src, Some(std::path::PathBuf::from("examples/interop/test_modules"))).unwrap();
    interp.add_python_search_dir(std::path::PathBuf::from("examples"));
    interp.add_python_search_dir(std::path::PathBuf::from("examples/interop/test_modules"));
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// テストソースを実行し、最初の `raise` で発生した例外を返すテストヘルパー。例外がなければ `Ok(None)`。
fn run_exc(src: &str) -> Result<Option<RaisedError>, String> {
    let (stmts, mut interp) = prepare(src)?;
    for stmt in &stmts {
        match interp.exec(stmt) {
            Ok(ExecResult::Raise(raised)) => return Ok(Some(raised)),
            Ok(_) => {}
            Err(e) if e == RAISE_SENTINEL => return Ok(interp.take_current_exception()),
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// arithmetic のテスト。

/// `val` が `Str(expected)` であることを表明するテストヘルパー。
fn assert_str(val: Value, expected: &str) {
    if let Value::Str(s) = val {
        assert_eq!(&*s, expected);
    } else {
        panic!("expected Str({:?}), got {:?}", expected, val);
    }
}

/// `val` が `Int(expected)` であることを表明するテストヘルパー。
fn assert_int(val: Value, expected: i64) {
    if let Value::Int(n) = val {
        assert_eq!(n, expected);
    } else {
        panic!("expected Int({}), got {:?}", expected, val);
    }
}

/// `val` が `List([Int(...)])` であることを表明するテストヘルパー。各要素の整数値を検証する。
fn assert_int_list(val: Value, expected: &[i64]) {
    if let Value::List(rc) = val {
        let list = rc.borrow();
        assert_eq!(list.len(), expected.len(), "list length mismatch");
        for (i, (v, e)) in list.iter().zip(expected.iter()).enumerate() {
            if let Value::Int(n) = v {
                assert_eq!(n, e, "list[{}] mismatch", i);
            } else {
                panic!("list[{}]: expected Int({}), got {:?}", i, e, v);
            }
        }
    } else {
        panic!("expected List, got {:?}", val);
    }
}

mod basics;
mod control_flow;
mod functions;
mod classes;
mod instances;
mod exceptions;
mod iterator;
mod collections;
mod callables;
mod indexing;
// ⚠ `import[py-int]` を実際に動かすので `native` 限定（評価コア切り出し #6）。
//   評価コアビルドでは `py_interop` がスタブなので必ず失敗する。
#[cfg(feature = "native")]
mod pyobject;
mod expressions;
mod enum_defaults;
mod file_io;
mod primitives;
mod set_type;
mod async_tests;
mod events_external;
mod unpacking;
mod mustbe;
mod hashing;
mod alias;
mod modules;

/// A 軸（呼び先の同定）の跨ファイル不変条件を固定するテスト（#22-d）。
///
/// 組み込み呼び出しの判断は **VM コンパイラ（`is_vm_builtin`）と
/// インタプリタ（`eval_builtin_evaled`）の 2 箇所**に分かれている。
/// 集合がずれると `CallBuiltin` を発行したのに実行側が `None` を返し、
/// **`NameError` で落ちる**（しかも VM 経路だけ＝off/auto 不一致になる）。
///
/// この系列では「同じ判断が 2 箇所にある」ことが繰り返し実バグを生んだ
/// （#22-a `JsProcFn` / #22-b `AsyncManager` / #22-c cs ブリッジ）。
/// 畳めない重複はテストで固定する。
mod a_axis_invariants {
    use crate::interpreter::Interpreter;

    /// 型検査の「実行時が宣言なしで解決する名前」の表（`type_check::names::RUNTIME_BUILTIN_NAMES`）が
    /// 実行時とずれていないか（フェーズ10 10-12）。**両方向**を見る。
    /// ⚠ 表に無い組み込みがあると、正しいプログラムが静的に「未定義」になる（偽の誤り）。
    /// ⚠ 表に実行時に無い名前が残ると、未定義の名前を見逃す。
    #[test]
    fn checker_knows_every_runtime_builtin_name() {
        use crate::type_check::names::{is_runtime_builtin_name, RUNTIME_BUILTIN_NAMES};
        let mut interp = Interpreter::new();
        let scope = Interpreter::builtin_global_scope(&crate::interpreter::Value::None);
        // ① 組み込みの大域（型・例外・`Signal`・`EventLoop` …）。足りない名前は全部まとめて言う。
        let mut missing: Vec<&String> =
            scope.iter().map(|(n, _)| n).filter(|n| !is_runtime_builtin_name(n)).collect();
        missing.sort();
        assert!(missing.is_empty(), "組み込みの大域が型検査の表に無い: {missing:?}");
        // ② VM が `CallBuiltin` を出す組み込み関数
        for name in crate::vm::compiler::VM_BUILTIN_NAMES {
            assert!(is_runtime_builtin_name(name), "組み込み関数 '{name}' が型検査の表に無い");
        }
        // ③ 組み込み関数の表（`eval/builtins.rs` の `"name" =>` の腕）
        for line in include_str!("../eval/builtins.rs").lines() {
            let t = line.trim_start();
            let Some(head) = t.strip_prefix('"').and_then(|_| t.split("=>").next()) else {
                continue;
            };
            if !t.contains("=>") {
                continue;
            }
            for alt in head.split('|') {
                let n = alt.trim().trim_matches('"');
                if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    assert!(is_runtime_builtin_name(n), "組み込み関数 '{n}' が型検査の表に無い");
                }
            }
        }
        // ④ 逆向き: 表の名前は実行時に解決できる
        for name in RUNTIME_BUILTIN_NAMES {
            let known = scope.contains_key(*name) || interp.eval_builtin_evaled(name, Vec::new()).is_some();
            assert!(known, "型検査の表の '{name}' を実行時が知らない（古い名前が残っている）");
        }
    }

    /// 組み込み関数の宣言（`src/built_in_stab/builtins.ars`）が実行時とずれていないか。**両方向**を見る。
    /// ⚠ 宣言に実行時に無い関数があると、型検査が NameError になる呼び出しに型を付け、補完が
    ///   存在しない関数を勧める（以前の拡張用 `builtins.ars` は `ord` / `abs` など 18 個を並べていた）。
    /// ⚠ 実行時の組み込み関数が宣言に無いと、その呼び出しに型が付かず、hover にも出ない。
    #[test]
    fn builtins_ars_matches_the_runtime() {
        let mut interp = Interpreter::new();
        let scope = Interpreter::builtin_global_scope(&crate::interpreter::Value::None);
        let declared = crate::type_check::builtins::declared_names();
        assert!(!declared.is_empty(), "builtins.ars が読めていない（構文エラー？）");
        // ① 宣言した名前は実行時に解決できる（組み込み関数・型の変換・組み込みのクラス）
        for name in &declared {
            let known = scope.contains_key(name.as_str())
                || interp.eval_builtin_evaled(name, Vec::new()).is_some()
                || match interp.call_type_by_name_evaled(name, Vec::new()) {
                    Ok(_) => true,
                    Err(e) => !e.contains("is not callable"),
                };
            assert!(known, "builtins.ars の '{name}' を実行時が知らない（実在しない関数を宣言している）");
        }
        // ② 実行時の組み込み関数の表（`eval/builtins.rs` の `"name" =>` の腕）は全部宣言されている
        for line in include_str!("../eval/builtins.rs").lines() {
            let t = line.trim_start();
            let Some(head) = t.strip_prefix('"').and_then(|_| t.split("=>").next()) else {
                continue;
            };
            if !t.contains("=>") {
                continue;
            }
            for alt in head.split('|') {
                let n = alt.trim().trim_matches('"');
                if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    assert!(declared.iter().any(|d| d == n), "組み込み関数 '{n}' が builtins.ars に無い");
                }
            }
        }

        // ③ 組み込みの `enum`: 宣言の要素と値が実行時（`built_in_types.rs`）と同じ
        let enums = crate::type_check::builtins::declared_enums();
        for (name, variants) in &enums {
            for (variant, value) in variants {
                let got = super::eval_expr(&format!("{name}.{variant}.value"));
                assert!(
                    matches!((&got, value), (crate::interpreter::Value::Int(g), Some(v)) if g == v),
                    "builtins.ars の {name}.{variant} = {value:?} が実行時の値 {got:?} と違う"
                );
            }
        }
        // ④ 逆向き: 実行時の組み込みの `enum`（メンバーの `enum_of` が自分の名前を指すクラス）は
        //    全部宣言されている。要素の数も揃える（宣言に無い要素は静的に「無い」と言われてしまう）。
        let mut runtime_enums = 0;
        for (name, var) in scope.iter() {
            let is_enum = matches!(var.get_value(), crate::interpreter::Value::Class(c)
                if c.class_vars.values().any(|m| matches!(m, crate::interpreter::Value::Instance(i)
                    if i.borrow().class.enum_of.as_deref() == Some(name.as_str()))));
            if !is_enum {
                continue;
            }
            runtime_enums += 1;
            let Some((_, variants)) = enums.iter().find(|(n, _)| n == name) else {
                panic!("実行時の組み込みの enum '{name}' が builtins.ars に無い");
            };
            let Some(crate::interpreter::Value::Class(cls)) = scope.get(name.as_str()).map(|v| v.get_value()) else {
                panic!("組み込みの enum '{name}' が実行時にクラスとして登録されていない");
            };
            assert_eq!(cls.class_vars.len(), variants.len(), "enum '{name}' の要素の数が実行時と違う");
        }
        // ⚠ 判定が 1 件も当たらないと④は空回りで緑になる。宣言の数と揃うことも見る。
        assert_eq!(runtime_enums, enums.len(), "実行時の組み込みの enum の数が builtins.ars と違う（判定の空回り？）");

        // ⑤ `FileObject` のメソッド: 宣言したものは実行時にある／実行時のもの（`exec_file_method` の腕）は宣言されている
        let methods: Vec<String> = crate::type_check::builtins::declared_methods("FileObject")
            .into_iter()
            .filter(|m| !m.starts_with("__"))
            .collect();
        assert!(!methods.is_empty(), "builtins.ars に FileObject のメソッドが無い");
        let fd = std::rc::Rc::new(std::cell::RefCell::new(crate::interpreter::FileData {
            path: String::new(),
            mode: crate::interpreter::FileOpenModeRust::Read,
            byte_mode: crate::interpreter::ByteModeRust::Text,
            content: Vec::new(),
            pointer: 0,
            is_closed: true,
            file_handle: None,
        }));
        for m in &methods {
            let res = interp.exec_file_method(fd.clone(), m, &[]);
            assert!(
                !matches!(&res, Err(e) if e.contains("has no method")),
                "builtins.ars の FileObject.{m} を実行時が知らない"
            );
        }
        let src = include_str!("../classes/object_methods.rs");
        let body = src.split("fn exec_file_method").nth(1).expect("exec_file_method");
        let body = body.split("pub(crate) fn ").next().unwrap_or(body);
        for line in body.lines() {
            let t = line.trim_start();
            if !t.starts_with('"') || !t.contains("=>") {
                continue;
            }
            for alt in t.split("=>").next().unwrap_or("").split('|') {
                let n = alt.trim().trim_matches('"');
                if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    assert!(methods.iter().any(|d| d == n), "FileObject.{n} が builtins.ars に無い");
                }
            }
        }
    }

    #[test]
    fn vm_builtin_names_are_all_handled() {
        let mut interp = Interpreter::new();
        for name in crate::vm::compiler::VM_BUILTIN_NAMES {
            // 引数は空でよい。ここで見たいのは「その名前を知っているか」だけで、
            // 引数不一致は `Some(Err(..))` になる（`None` は「知らない名前」を意味する）。
            let handled = interp.eval_builtin_evaled(name, Vec::new()).is_some();
            assert!(
                handled,
                "is_vm_builtin に '{name}' があるが eval_builtin_evaled が扱っていない。\n\
                 VM が CallBuiltin を発行して実行時 NameError になる（off/auto 不一致）。\n\
                 eval_builtin_evaled にアームを足すか、VM_BUILTIN_NAMES から外すこと。"
            );
        }
    }
}

/// 「同じ演算は**書き方**で特化が変わらない」を固定するテスト（#2b）。
///
/// `x <op>= e` と `x = x <op> e` は同じ二項演算だが、型特化の判断は
/// **`Expr::BinOp`（infer.rs）と `Stmt::CompoundAssign`（stmt/check.rs）の 2 箇所**にある。
/// 片方が注釈を焼き忘れると複合代入だけが汎用 `Op::Bin` に落ちる（実測 1.9x 遅い）。
/// 実際 #2b 着手時点ではその状態だった。
///
/// 挙動は変わらない（特化 op は想定外型なら汎用へフォールバックする）ため
/// `compare_vm_modes.ps1` では検知できない。命令列を直接見て固定する。
mod bin_specialization_invariants {
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::op::Op;

    /// ソース中の関数 `name` をコンパイルして op 列を返す。
    fn compile(src: &str, name: &str) -> Vec<Op> {
        let tokens = Lexer::new(src, "").tokenize();
        let mut stmts = Parser::new(tokens, None).parse_program().unwrap();
        // ⚠ 配線は 1 箇所（#88）。
        let (_errs, _warns, annots) =
            crate::interpreter::resolver::resolve_and_annotate(&mut stmts);
        let annots = std::rc::Rc::new(annots);
        for s in &stmts {
            if let crate::ast::Stmt::FnDef { name: n, params, body, .. } = s {
                if n == name {
                    return crate::vm::compile_fn(params, body, annots, &[], &[])
                        .unwrap_or_else(|| panic!("`{name}` が VM コンパイルできなかった"))
                        .code;
                }
            }
        }
        panic!("`{name}` が見つからない");
    }

    /// 型特化に乗らなかった二項演算 op の数。
    ///
    /// ⚠ `Op::Bin` だけを数えるのでは**不十分**（負の対照で確認済み）。融合だけ効いて特化が
    /// 落ちると `BinLocalConst`/`BinLocalLocal` になり、`Op::Bin` は 0 のまま通ってしまう。
    fn unspecialized_bins(code: &[Op]) -> usize {
        code.iter()
            .filter(|o| {
                matches!(
                    o,
                    Op::Bin(_) | Op::BinLocalLocal(..) | Op::BinLocalConst(..)
                )
            })
            .count()
    }

    #[test]
    fn compound_assign_is_specialized_like_plain_assign() {
        // int / float それぞれで、明示形と複合代入形が同じ特化状態になることを見る。
        let src = "\
fn plain_i(let n: int) -> int:
    mut acc = 0
    acc = acc + 3
    return acc

fn comp_i(let n: int) -> int:
    mut acc = 0
    acc += 3
    return acc

fn plain_f(let n: int) -> float:
    mut acc = 0.0
    acc = acc + 1.5
    return acc

fn comp_f(let n: int) -> float:
    mut acc = 0.0
    acc += 1.5
    return acc
";
        for (plain, comp) in [("plain_i", "comp_i"), ("plain_f", "comp_f")] {
            let pc = compile(src, plain);
            let cc = compile(src, comp);
            assert_eq!(
                unspecialized_bins(&pc),
                0,
                "{plain} が特化に乗っていない（テストの前提が崩れている）: {pc:?}"
            );
            // 本命: 両者は同じ演算なので**命令列が完全に一致**するはず。
            // 特化が落ちれば op 種別が、融合が落ちれば命令数が食い違って検知できる。
            // `Op` は `PartialEq` を導出していないので Debug 表現で比較する。
            assert_eq!(
                format!("{pc:?}"),
                format!("{cc:?}"),
                "`{comp}`（`x <op>= e`）と `{plain}`（`x = x <op> e`）の命令列が違う。\n\
                 同じ演算なので同じ命令列になるべき。型検査（stmt/check.rs の CompoundAssign）が\n\
                 binop_kind を焼いているか、VM コンパイラが specialized_bin_kind_slot と\n\
                 emit_bin_fused_slot を通しているかを確認すること。"
            );
        }
    }
}

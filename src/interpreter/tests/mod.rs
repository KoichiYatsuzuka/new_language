// tests/mod.rs — インタープリタ単体テストのモジュール束ね。
// 共通テストヘルパー(run/eval_expr/run_get/run_exc/assert_* 等)を定義し、機能別サブモジュールを宣言する。

use super::*;
use crate::ast::Stmt;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// テストソースを字句解析・構文解析・実行する。エラーがあれば `Err` を返す。
fn run(src: &str) -> Result<(), String> {
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program()?;
    super::resolver::resolve_program(&mut stmts);
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt)?;
    }
    Ok(())
}

/// 単一の式文を評価して `Value` を返すテストヘルパー。
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

/// テストソースを実行して変数 `var` の値を返すテストヘルパー。
fn run_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().unwrap();
    super::resolver::resolve_program(&mut stmts);
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// py-int テスト用: examples/ ディレクトリを Python 検索パスに追加して実行する
fn run_py_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program().unwrap();
    super::resolver::resolve_program(&mut stmts);
    let mut interp = Interpreter::new();
    interp.add_python_search_dir(std::path::PathBuf::from("examples"));
    interp.add_python_search_dir(std::path::PathBuf::from("examples/interop/test_modules"));
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// テストソースを実行し、最初の `raise` で発生した例外を返すテストヘルパー。例外がなければ `Ok(None)`。
fn run_exc(src: &str) -> Result<Option<RaisedError>, String> {
    let tokens = Lexer::new(src, "").tokenize();
    let mut stmts = Parser::new(tokens, None).parse_program()?;
    super::resolver::resolve_program(&mut stmts);
    let mut interp = Interpreter::new();
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
mod alias;

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
    use crate::type_check::TypeChecker;
    use crate::vm::op::Op;

    /// ソース中の関数 `name` をコンパイルして op 列を返す。
    fn compile(src: &str, name: &str) -> Vec<Op> {
        let tokens = Lexer::new(src, "").tokenize();
        let mut stmts = Parser::new(tokens, None).parse_program().unwrap();
        crate::interpreter::resolver::resolve_program(&mut stmts);
        let (_errs, annots) = TypeChecker::check_and_annotate(&stmts);
        let annots = std::rc::Rc::new(annots);
        for s in &stmts {
            if let crate::ast::Stmt::FnDef { name: n, params, body, .. } = s {
                if n == name {
                    return crate::vm::compile_fn(params, body, annots)
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

// tests/mod.rs — インタープリタ単体テストのモジュール束ね。
// 共通テストヘルパー(run/eval_expr/run_get/run_exc/assert_* 等)を定義し、機能別サブモジュールを宣言する。

use super::*;
use crate::ast::Stmt;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// テストソースを字句解析・構文解析・実行する。エラーがあれば `Err` を返す。
fn run(src: &str) -> Result<(), String> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program()?;
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
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// py-int テスト用: examples/ ディレクトリを Python 検索パスに追加して実行する
fn run_py_get(src: &str, var: &str) -> Value {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program().unwrap();
    let mut interp = Interpreter::new();
    interp.add_python_search_dir(std::path::PathBuf::from("examples"));
    interp.add_python_search_dir(std::path::PathBuf::from("examples/test_modules"));
    for stmt in &stmts {
        let _ = interp.exec(stmt).unwrap();
    }
    interp.get_val(var).unwrap()
}

/// テストソースを実行し、最初の `raise` で発生した例外を返すテストヘルパー。例外がなければ `Ok(None)`。
fn run_exc(src: &str) -> Result<Option<RaisedError>, String> {
    let tokens = Lexer::new(src, "").tokenize();
    let stmts = Parser::new(tokens, None).parse_program()?;
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
        assert_eq!(s, expected);
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
mod unpacking;
mod mustbe;

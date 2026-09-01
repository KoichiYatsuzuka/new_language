// python_converter/mod.rs — rustpython-parser の AST を Arrow の AST に変換するサブシステムの束ね。
// 公開エントリポイント convert_python_source を保持し、役割別サブモジュール
// (statements/classes/decorators/supers/param_rewrite/expressions/annotations/utils)を宣言する。

use rustpython_parser::{ast as py, Parse};

use crate::ast::Stmt;

// ---------------------------------------------------------------------------
// 公開エントリポイント
// ---------------------------------------------------------------------------

/// Python ソースコード文字列を解析し、tl の `Stmt` リストに変換する。
pub fn convert_python_source(source: &str, filename: &str) -> Result<Vec<Stmt>, String> {
    let ast = py::Suite::parse(source, filename).map_err(|e| format!("{filename}: {e}"))?;
    // モジュール本体も 1 つのスコープ（パラメータは無い）。
    convert_scope(&ast, filename, &[])
}


mod statements;
mod classes;
mod decorators;
mod supers;
mod param_rewrite;
mod expressions;
mod annotations;
mod utils;
pub(crate) use statements::*;
pub(crate) use classes::*;
pub(crate) use decorators::*;
pub(crate) use supers::*;
pub(crate) use param_rewrite::*;
pub(crate) use expressions::*;
pub(crate) use annotations::*;
pub(crate) use utils::*;

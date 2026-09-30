//! 組み込み関数の型。宣言は `src/built_in_stab/builtins.ars` **1 本だけ**で、ここはそれを読む。
//!
//! # なぜ宣言ファイルを型検査器が読むのか
//!
//! 以前の組み込み関数の型は 2 か所に分かれていた:
//! - 型検査器 … `range` / `len` のシグネチャを `TypeChecker::new` に手書き。他（`open` / `repr` …）は
//!   型が無く、呼び出しの結果がすべて `Unresolved`（＝何でも通る）だった。
//! - VS Code 拡張 … `vscode-extension/builtins.ars` を hover 用に読むだけ。実行時に無い関数（`ord` /
//!   `abs` …）まで並び、`code` が予約語になってからは丸ごと読めなくなっていた。
//! ⇒ 宣言を 1 本にして、型検査器（`arrow.exe` と拡張の両方）と拡張の表示が同じものを見るようにした。
//!   拡張は wasm の `ar_builtins`（[`SOURCE`]）から同じ本文を受け取る。
//!
//! # 宣言の読み方（`builtins.ars` 冒頭の規則と同じ）
//!
//! - 戻り値の注釈が無い関数は、型が静的に決まらない（`next` / `parse_ar` …）→ 型を付けない。
//! - 名前が型の名前（`int` / `list` …・[`is_conversion`]）の関数は型の変換 → ここからは型を付けない。
//!   `int` / `float` / `str` / `bool` は「型の値を呼ぶとその型」の規則（`call_check`）が付ける。
//! - 仮引数を検査に使うのは、大域の名前を占有する [`GLOBAL_FNS`] だけ（`TypeChecker::new`）。
//!   他の関数の仮引数は表示用。
//!
//! ⚠ 利用者が同じ名前の関数・変数を宣言したら、そちらが勝つ（呼ぶ側 `builtin_fn_return` が先に見る）。

use std::collections::HashMap;

use super::types::{FnTypeParam, InferredType};
use crate::ast::Stmt;

/// 組み込み関数の宣言の本文（`builtins.ars`）。VS Code 拡張へもこの本文をそのまま渡す。
pub const SOURCE: &str = include_str!("../built_in_stab/builtins.ars");

/// 大域の名前を**占有する**組み込み関数。型検査器はこの名前を大域の変数（関数型）として登録するので、
/// 利用者が同じ名前を `let` すると再宣言の誤りになる（実行時も既に占有している名前だけ・`TypeChecker::new`）。
pub(super) const GLOBAL_FNS: [&str; 2] = ["range", "len"];

/// 1 つの組み込み関数の宣言。
pub(crate) struct BuiltinFn {
    /// 仮引数（名前・型・既定値の有無）。
    pub params: Vec<FnTypeParam>,
    /// 戻り値の型。注釈が無いもの（静的に決まらない）は `None`。
    pub ret: Option<InferredType>,
}

thread_local! {
    /// `builtins.ars` を解析した表（スレッドごとに 1 回だけ作る）。
    static TABLE: HashMap<String, BuiltinFn> = parse_table();
}

/// `builtins.ars` を解析して「名前 → 宣言」の表を作る。
///
/// ⚠ 解析に失敗したら空の表を返す（組み込みに型が付かないだけで、検査は続けられる）。
///   ファイルが壊れていないことは単体テスト（`builtins_ars_parses`）が守る。
fn parse_table() -> HashMap<String, BuiltinFn> {
    let tokens = crate::lexer::Lexer::new(SOURCE, "<builtins>").tokenize();
    let stmts = match crate::parser::Parser::new(tokens, None).parse_program() {
        Ok(stmts) => stmts,
        Err(e) => {
            debug_assert!(false, "builtins.ars failed to parse: {e}");
            return HashMap::new();
        }
    };
    let mut table = HashMap::new();
    for stmt in stmts {
        let Stmt::FnDef { name, params, return_type, .. } = stmt else {
            continue;
        };
        let params = params
            .iter()
            .map(|p| FnTypeParam {
                name: p.name.clone(),
                mutable: p.mutable,
                ty: p
                    .type_ann
                    .as_deref()
                    .and_then(InferredType::from_ann)
                    .unwrap_or(InferredType::Any),
                has_default: p.default.is_some(),
            })
            .collect();
        let ret = return_type.as_deref().and_then(InferredType::from_ann);
        table.insert(name, BuiltinFn { params, ret });
    }
    table
}

/// 名前が型の名前（`int` / `uint` / `list` / `set` …）か。そういう名前の組み込み関数は型の変換。
///
/// 型の表（`InferredType::from_ann`）がその名前を**型として**知っているかで決める
/// （知らない名前は `NamedInstance` になる。`path` / `slice` はそちらなので関数として型が付く）。
pub(crate) fn is_conversion(name: &str) -> bool {
    !matches!(InferredType::from_ann(name), None | Some(InferredType::NamedInstance(_)))
}

/// 組み込み関数 `name` を呼んだ結果の型。組み込みでない・型が静的に決まらない・型の変換なら `None`。
pub(super) fn return_type(name: &str) -> Option<InferredType> {
    if is_conversion(name) {
        return None;
    }
    TABLE.with(|t| t.get(name).and_then(|f| f.ret.clone()))
}

/// 組み込み関数 `name` の関数型（仮引数つき）。大域に登録する [`GLOBAL_FNS`] 用。
pub(super) fn fn_type(name: &str) -> Option<InferredType> {
    TABLE.with(|t| {
        t.get(name).map(|f| InferredType::Function {
            params: Some(f.params.clone()),
            return_type: Box::new(f.ret.clone().unwrap_or(InferredType::Unresolved)),
        })
    })
}

/// `builtins.ars` が宣言している関数の名前（宣言と実行時の突き合わせ用）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn declared_names() -> Vec<String> {
    TABLE.with(|t| {
        let mut names: Vec<String> = t.keys().cloned().collect();
        names.sort();
        names
    })
}

//! 組み込み関数・組み込みの型の宣言。宣言は `src/built_in_stab/builtins.ars` **1 本だけ**で、ここはそれを読む。
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
//! - 仮引数で呼び出しの引数を検査する（[`call_decl`]・`call_check` の `check_builtin_call`）。
//!   [`GLOBAL_FNS`]（`range` / `len`）は大域の関数型として登録するので、その経路（`check_fn_type_call`）で検査される。
//! - `let ...: T`（`print` / `zip`）は**個数自由の位置引数**（[`BuiltinFn::variadic`]）。
//!
//! ⚠ 利用者が同じ名前の関数・変数を宣言したら、そちらが勝つ（呼ぶ側 `builtin_fn_return` が先に見る）。
//!
//! # 組み込みの型（`enum` / `class`）
//!
//! 実行時の専用の値（`Value::FileObject`）や組み込みの列挙（`FileOpenMode` …）は、以前は型検査器に
//! 定義が無く、`FileOpenMode.read` も `f.read()` も `Unresolved`（何でも通る）だった。
//! `builtins.ars` の `enum` / `class` を [`type_decls`] で渡し、`TypeChecker::new` がレジストリへ
//! （`TypeRegistryBuilder::collect_builtin_types`・Arrow のクラスとしては扱わない）、`enum` の名前を
//! 大域へ登録する。これで要素・メソッドの存在、メソッドの引数、`.value` の型が検査される。

use std::collections::HashMap;

use super::types::{FnTypeParam, InferredType};
use crate::ast::Stmt;

/// 組み込み関数の宣言の本文（`builtins.ars`）。VS Code 拡張へもこの本文をそのまま渡す。
pub const SOURCE: &str = include_str!("../built_in_stab/builtins.ars");

/// 大域の名前を**占有する**組み込み関数。型検査器はこの名前を大域の変数（関数型）として登録するので、
/// 利用者が同じ名前を `let` すると再宣言の誤りになる（実行時も既に占有している名前だけ・`TypeChecker::new`）。
pub(super) const GLOBAL_FNS: [&str; 2] = ["range", "len"];

/// 1 つの組み込み関数の宣言。
#[derive(Clone)]
pub(crate) struct BuiltinFn {
    /// 仮引数（名前・型・既定値の有無）。`let ...: T` は含まない（[`Self::variadic`]）。
    pub params: Vec<FnTypeParam>,
    /// `let ...: T` の `T`。組み込みでは「位置引数を何個でも受け取る」（`print(a, b)`）。
    /// ⚠ Arrow の可変長引数（呼ぶ側が `f(... = a, b)`）とは呼び方が違う。キーワード引数は受け取らない。
    pub variadic: Option<InferredType>,
    /// 戻り値の型。注釈が無いもの（静的に決まらない）は `None`。
    pub ret: Option<InferredType>,
}

/// `builtins.ars` を解析した結果。
struct Declarations {
    /// 組み込み関数（名前 → 宣言）。
    fns: HashMap<String, BuiltinFn>,
    /// 組み込みの型の宣言（`enum` / `class` の文そのもの）。
    types: Vec<Stmt>,
}

thread_local! {
    /// `builtins.ars` を解析した表（スレッドごとに 1 回だけ作る）。
    static TABLE: Declarations = parse_table();
}

/// `builtins.ars` を解析して「名前 → 宣言」の表を作る。
///
/// ⚠ 解析に失敗したら空の表を返す（組み込みに型が付かないだけで、検査は続けられる）。
///   ファイルが壊れていないことは単体テスト（`builtins_ars_parses`）が守る。
fn parse_table() -> Declarations {
    let mut table = Declarations { fns: HashMap::new(), types: Vec::new() };
    let tokens = crate::lexer::Lexer::new(SOURCE, "<builtins>").tokenize();
    let stmts = match crate::parser::Parser::new(tokens, None).parse_program() {
        Ok(stmts) => stmts,
        Err(e) => {
            debug_assert!(false, "builtins.ars failed to parse: {e}");
            return table;
        }
    };
    for stmt in stmts {
        let Stmt::FnDef { name, params, return_type, .. } = stmt else {
            if matches!(stmt, Stmt::EnumDef { .. } | Stmt::ClassDef { .. }) {
                table.types.push(stmt);
            }
            continue;
        };
        let variadic = params.iter().find(|p| p.variadic).map(|p| {
            p.type_ann.as_deref().and_then(InferredType::from_ann).unwrap_or(InferredType::Any)
        });
        let params = params
            .iter()
            .filter(|p| !p.variadic)
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
        table.fns.insert(name, BuiltinFn { params, variadic, ret });
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
    TABLE.with(|t| t.fns.get(name).and_then(|f| f.ret.clone()))
}

/// 組み込み関数 `name` の呼び出しの検査に使う宣言。組み込みでない・型の変換なら `None`。
///
/// ⚠ 型の変換（`int(x)` …）は検査しない（引数の受け方が型ごとに違い、宣言は表示用・[`is_conversion`]）。
pub(crate) fn call_decl(name: &str) -> Option<BuiltinFn> {
    if is_conversion(name) {
        return None;
    }
    TABLE.with(|t| t.fns.get(name).cloned())
}

/// 組み込み関数 `name` の関数型（仮引数つき）。大域に登録する [`GLOBAL_FNS`] 用。
pub(super) fn fn_type(name: &str) -> Option<InferredType> {
    TABLE.with(|t| {
        t.fns.get(name).map(|f| InferredType::Function {
            params: Some(f.params.clone()),
            return_type: Box::new(f.ret.clone().unwrap_or(InferredType::Unresolved)),
        })
    })
}

/// `builtins.ars` が宣言している関数の名前（宣言と実行時の突き合わせ用）。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn declared_names() -> Vec<String> {
    TABLE.with(|t| {
        let mut names: Vec<String> = t.fns.keys().cloned().collect();
        names.sort();
        names
    })
}

/// 組み込みの型の宣言（`builtins.ars` の `enum` / `class` の文）。`TypeChecker::new` がレジストリと大域へ登録する。
pub(super) fn type_decls() -> Vec<Stmt> {
    TABLE.with(|t| t.types.clone())
}

/// 組み込みの `enum` の宣言（名前・要素と値）。宣言と実行時の突き合わせ用。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn declared_enums() -> Vec<(String, Vec<(String, Option<i64>)>)> {
    TABLE.with(|t| {
        t.types
            .iter()
            .filter_map(|s| match s {
                Stmt::EnumDef { name, variants, .. } => Some((
                    name.clone(),
                    variants
                        .iter()
                        .map(|(v, e)| {
                            let value = match e {
                                Some(crate::ast::Expr::Int(n)) => Some(*n),
                                _ => None,
                            };
                            (v.clone(), value)
                        })
                        .collect(),
                )),
                _ => None,
            })
            .collect()
    })
}

/// 組み込みの `class` の宣言したメソッド名（クラス名 → メソッド名）。宣言と実行時の突き合わせ用。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn declared_methods(class: &str) -> Vec<String> {
    TABLE.with(|t| {
        t.types
            .iter()
            .find_map(|s| match s {
                Stmt::ClassDef { name, body, .. } if name == class => Some(
                    body.iter()
                        .filter_map(|m| match m {
                            Stmt::FnDef { name, .. } => Some(name.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    })
}

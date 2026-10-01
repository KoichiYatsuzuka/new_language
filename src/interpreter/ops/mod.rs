// ops/mod.rs — 演算・比較・真偽値・表示サブシステムのモジュール束ね。
//
// `Value` に対する演算・表示・型名取得などの基本操作を実装する。頻繁に呼ばれる共通ユーティリティ群。
// 共有の自由ヘルパー(format_fn_params)を保持し、役割別サブモジュール
// (typecheck/display/operators/equality/hash)を宣言する。

use crate::ast::Param;

/// 関数パラメータリストを `(name: Type, name2)` 形式の文字列に変換する。
/// `self` パラメータは除外する。
fn format_fn_params(params: &[Param]) -> String {
    params
        .iter()
        .filter(|p| p.name != "self")
        .map(|p| {
            if let Some(t) = &p.type_ann {
                format!("{}: {}", p.name, t)
            } else {
                p.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 整数の `//`。CPython と同じく商を**負の無限大の方向へ**丸める。
///
/// ⚠ `div_euclid` ではない。ユークリッド除算は余りを常に非負にするので、割る数が負のとき
///   CPython と食い違う（`-7 // -2` が `4`。CPython は `3`）。ネイティブ codegen の `@_tl_idiv` と同じ式。
/// ⚠ ゼロ除算は呼び出し側で弾くこと（エラーの文言を `apply_binop` の 1 か所に保つため）。
/// `i64::MIN // -1` は `+` などのあふれと同じく折り返す（パニックしない）。
#[inline]
pub(crate) fn py_floor_div(a: i64, b: i64) -> i64 {
    let q = a.wrapping_div(b);
    let r = a.wrapping_rem(b);
    if r != 0 && ((r < 0) != (b < 0)) { q - 1 } else { q }
}

/// 整数の `%`。CPython と同じく余りは**割る数と同じ符号**になる（`7 % -2` は `-1`）。
///
/// ⚠ `rem_euclid` ではない（常に非負になり、割る数が負のとき CPython と食い違う）。
///   ネイティブ codegen の `@_tl_imod` と同じ式。ゼロ除算は呼び出し側で弾くこと。
#[inline]
pub(crate) fn py_mod(a: i64, b: i64) -> i64 {
    let r = a.wrapping_rem(b);
    if r != 0 && ((r < 0) != (b < 0)) { r + b } else { r }
}


pub(crate) mod typecheck;
mod display;
mod operators;
mod equality;
pub(crate) mod hash;

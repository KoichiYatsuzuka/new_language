// ops/mod.rs — 演算・比較・真偽値・表示サブシステムのモジュール束ね。
//
// `Value` に対する演算・表示・型名取得などの基本操作を実装する。頻繁に呼ばれる共通ユーティリティ群。
// 共有の自由ヘルパー(format_fn_params)を保持し、役割別サブモジュール
// (typecheck/display/operators/equality)を宣言する。

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


mod typecheck;
mod display;
mod operators;
mod equality;

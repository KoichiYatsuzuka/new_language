// python_converter/hoist.rs — 式コンテキストから囲みスコープへ文を注入する機構（INF-B / INF-C）。
//
// ★ Python には「式の中で文をやる」形がいくつかある:
//     (x := expr)            walrus（項目 23）
//     lambda a: a + 1        無名関数（項目 26。Arrow には名前付き `fn` しか無い）
//     a = b = c              複数代入（項目 15。RHS を 1 回だけ評価したい）
//     a, b = t               タプル代入（U2）
//   いずれも「**この文の直前に補助文を置き、式はその結果を参照する**」形に脱糖できる。
//
// ⚠⚠ `convert_expr` に `&mut Vec<Stmt>` を足す案は**シグネチャ変更が 59 箇所に及ぶ**。
//   変換器は 1 スレッドで動くので、`supers.rs` / `param_rewrite.rs` と同じく
//   スレッドローカルのスタックで持つ（この 2 つが同じ理由で同じ方式を採っている）。
//
// 使い方（`convert_stmts` / `convert_scope` が面倒を見る）:
//   1. 文を 1 つ変換する前に `hoist_push()`
//   2. `convert_stmt(...)` を呼ぶ。中の `convert_expr` が `hoist_emit(...)` で補助文を積む
//   3. `hoist_pop()` で補助文を取り出し、**変換結果の前に**並べる
//
// ⚠ バッファは**文ごと**に積む。入れ子の本体（`if` の中など）は内側の
//   `convert_stmts` が自分のバッファを積むので、外側の文の補助文と混ざらない。

use std::cell::{Cell, RefCell};

use crate::ast::Stmt;

thread_local! {
    /// 文ごとの補助文バッファ（入れ子のためにスタック）。
    static HOIST: RefCell<Vec<Vec<Stmt>>> = const { RefCell::new(Vec::new()) };
    /// 一時変数の連番。モジュールごとに 0 から振り直す。
    static TEMP_SEQ: Cell<u32> = const { Cell::new(0) };
}

/// モジュールの変換を始めるときに状態を空にする。
///
/// ⚠ 変換が途中でエラー終了するとスタックが積まれたまま残るので、**入口で必ず**呼ぶ
/// （`convert_python_source`）。残骸が次のモジュールへ漏れると、無関係な文の前に
/// 補助文が差し込まれる。
pub(crate) fn reset_hoist() {
    HOIST.with(|h| h.borrow_mut().clear());
    TEMP_SEQ.with(|c| c.set(0));
}

/// 新しい補助文バッファを積む（1 文の変換を始める）。
pub(crate) fn hoist_push() {
    HOIST.with(|h| h.borrow_mut().push(Vec::new()));
}

/// 現在のバッファを取り出して閉じる（1 文の変換を終える）。
pub(crate) fn hoist_pop() -> Vec<Stmt> {
    HOIST.with(|h| h.borrow_mut().pop().unwrap_or_default())
}

/// 補助文を**現在の文の直前**に置くよう積む。
///
/// ⚠ バッファが無いときに黙って捨てると「脱糖したはずの文が消える」最悪の失敗形に
/// なるので、`false` を返して呼び出し側にエラーを出させる。
#[must_use]
pub(crate) fn hoist_emit(stmt: Stmt) -> bool {
    HOIST.with(|h| match h.borrow_mut().last_mut() {
        Some(buf) => {
            buf.push(stmt);
            true
        }
        None => false,
    })
}

/// 衝突しない一時変数名を作る（`__py_tmp_0` / `__py_lambda_1` …）。
///
/// ⚠ Python の識別子として**書けてしまう**名前なので、理屈のうえでは利用者の名前と
/// 衝突しうる。連番はモジュール単位で一意なので、同名を 2 回作ることはない。
pub(crate) fn next_temp_name(prefix: &str) -> String {
    TEMP_SEQ.with(|c| {
        let n = c.get();
        c.set(n + 1);
        format!("__py_{prefix}_{n}")
    })
}

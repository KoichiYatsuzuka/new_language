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

/// float の表示（`str` / `repr` / `print`）。CPython 3.12 の `float_repr`
/// （`PyOS_double_to_string(x, 'r', 0, Py_DTSF_ADD_DOT_0)`）と同じ（python_builtins_plan.md のタスク 1-5）。
///
/// 最短の往復表現の数字列（Rust の `{:e}` が出すものと同じ）を、10 進の小数点の位置 `decpt` で並べ直す:
///   - `decpt <= -4` か `decpt > 16` なら指数表記（`1e+16` / `1e-05`・指数は 2 桁以上・符号つき）
///   - それ以外は小数表記で、整数の値には `.0` を付ける（`100.0` / `0.0001`）
/// ⚠ 以前は指数表記を使わず、`1e20` が `100000000000000000000`（`.0` も無し）、`0.1 % 0.01` が
///   `0.000000000000000003469446951953614` と出ていた。`nan` も `NaN` だった。
pub(crate) fn py_float_repr(f: f64) -> String {
    py_float_fmt(f, true)
}

/// [`py_float_repr`] の本体。`add_dot_0` は整数の値に `.0` を付けるか（float は付ける・complex の成分は付けない）。
pub(crate) fn py_float_fmt(f: f64, add_dot_0: bool) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let sci = format!("{:e}", f.abs());
    let (mant, exp) = sci.split_once('e').expect("LowerExp always writes an exponent");
    let exp: i32 = exp.parse().expect("LowerExp exponent is an integer");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let decpt = exp + 1;
    let mut out = String::new();
    if f.is_sign_negative() {
        out.push('-');
    }
    if decpt <= -4 || decpt > 16 {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push_str(&format!("e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs()));
    } else if decpt <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-decpt) as usize));
        out.push_str(&digits);
    } else if (decpt as usize) < digits.len() {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    } else {
        out.push_str(&digits);
        out.push_str(&"0".repeat(decpt as usize - digits.len()));
        if add_dot_0 {
            out.push_str(".0");
        }
    }
    out
}

/// float の `//` と `%`（`(商, 余り)`）。CPython 3.12 の `_float_div_mod`（`Objects/floatobject.c`）を写したもの。
///
/// - 余りは `fmod`（Rust の `f64 % f64`）で出す。`a - floor(a/b)*b` は `a/b` の丸めを拾って端でずれる
///   （`0.1 % 0.01` が `0.0`、CPython は `3.469446951953614e-18`・`-1.0 % inf` が `nan`、CPython は `inf`）。
/// - 余りの符号を割る数に合わせる（`7.5 % -2.0` は `-0.5`）。余りが 0 なら割る数の符号つきの 0（`6.0 % -3.0` は `-0.0`）。
/// - 商は整数へ寄せる（`(a - mod) / b` は丸めで整数から少しずれることがある）。商が 0 なら真の商の符号つきの 0。
/// ⚠ ゼロ除算は呼び出し側で弾くこと（CPython の文言 `float floor division by zero` / `float modulo` を `apply_binop` に置く）。
/// ⚠ ネイティブ codegen の `@_tl_fdivmod`（`partial_compiler/llvm_codegen/mod.rs`）と同じ手順。片方だけ直さないこと。
#[inline]
pub(crate) fn py_float_div_mod(a: f64, b: f64) -> (f64, f64) {
    let mut m = a % b;
    let mut div = (a - m) / b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) {
            m += b;
            div -= 1.0;
        }
    } else {
        m = 0.0f64.copysign(b);
    }
    let q = if div != 0.0 {
        let fl = div.floor();
        if div - fl > 0.5 { fl + 1.0 } else { fl }
    } else {
        0.0f64.copysign(a / b)
    };
    (q, m)
}


pub(crate) mod typecheck;
mod display;
mod operators;
mod equality;
pub(crate) mod hash;

// vm/run/scalar_ops.rs — 数値の二項演算と真偽判定の高速パス。

use super::*;

/// int 型特化二項演算（plan A）。`apply_bin_fast` の (Int,Int) アームと同一意味論。
/// Add/Sub/Mul と比較のみ対応（Div/Mod/Pow/bit・ゼロ除算しうる op は `None`＝汎用へ委譲）。
#[inline]
pub(super) fn int_binop_specialized(x: i64, y: i64, op: &BinOp) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Int(x + y),
        BinOp::Sub => Value::Int(x - y),
        BinOp::Mul => Value::Int(x * y),
        // ゼロ除算は `None` を返して汎用パスへ委ねる。エラーメッセージ（`ZeroDivisionError: …` の
        // 3 種の文言）を `apply_binop` の一箇所に保つため、ここでは複製しない。
        BinOp::Div => {
            if y == 0 {
                return None;
            }
            Value::Float(x as f64 / y as f64)
        }
        BinOp::FloorDiv => {
            if y == 0 {
                return None;
            }
            Value::Int(crate::interpreter::ops::py_floor_div(x, y))
        }
        BinOp::Mod => {
            if y == 0 {
                return None;
            }
            Value::Int(crate::interpreter::ops::py_mod(x, y))
        }
        // 指数が非負なら整数冪、負なら float（`apply_binop` の Int/Int アームと同一）。
        BinOp::Pow => {
            if y >= 0 {
                Value::Int(x.pow(y as u32))
            } else {
                Value::Float((x as f64).powi(y as i32))
            }
        }
        BinOp::BitAnd => Value::Int(x & y),
        BinOp::BitOr => Value::Int(x | y),
        BinOp::BitXor => Value::Int(x ^ y),
        BinOp::LShift => Value::Int(x << y),
        BinOp::RShift => Value::Int(x >> y),
        BinOp::Lt => Value::Bool(x < y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::LtEq => Value::Bool(x <= y),
        BinOp::GtEq => Value::Bool(x >= y),
        BinOp::Eq => Value::Bool(x == y),
        BinOp::NotEq => Value::Bool(x != y),
        _ => return None,
    })
}

/// float 型特化二項演算（plan A）。`apply_bin_fast` の (Float,Float) アームと同一意味論。
#[inline]
pub(super) fn float_binop_specialized(x: f64, y: f64, op: &BinOp) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Float(x + y),
        BinOp::Sub => Value::Float(x - y),
        BinOp::Mul => Value::Float(x * y),
        // float の除算はゼロ検査なし（inf/NaN を返す）＝ `apply_binop` の Float/Float アームと同一。
        // `//` と `%` は特化しない（`gate_bin_kind` が許さない・汎用の `apply_binop` が CPython の手順で計算する）。
        BinOp::Div => Value::Float(x / y),
        BinOp::Pow => Value::Float(x.powf(y)),
        BinOp::Lt => Value::Bool(x < y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::LtEq => Value::Bool(x <= y),
        BinOp::GtEq => Value::Bool(x >= y),
        BinOp::Eq => Value::Bool(x == y),
        BinOp::NotEq => Value::Bool(x != y),
        _ => return None,
    })
}

/// int/float の算術・順序比較を高速パスで処理し、それ以外は `apply_binop_dyn` へ委譲する。
/// 高速パスは `apply_binop` の該当アームと同一セマンティクス。
#[inline]
pub(super) fn apply_bin_fast(
    interp: &mut Interpreter,
    op: &BinOp,
    a: Value,
    b: Value,
) -> Result<Value, String> {
    use Value::{Float, Int};
    let r = match (op, &a, &b) {
        (BinOp::Add, Int(x), Int(y)) => Int(*x + *y),
        (BinOp::Add, Float(x), Float(y)) => Float(*x + *y),
        (BinOp::Add, Int(x), Float(y)) => Float(*x as f64 + *y),
        (BinOp::Add, Float(x), Int(y)) => Float(*x + *y as f64),
        (BinOp::Sub, Int(x), Int(y)) => Int(*x - *y),
        (BinOp::Sub, Float(x), Float(y)) => Float(*x - *y),
        (BinOp::Sub, Int(x), Float(y)) => Float(*x as f64 - *y),
        (BinOp::Sub, Float(x), Int(y)) => Float(*x - *y as f64),
        (BinOp::Mul, Int(x), Int(y)) => Int(*x * *y),
        (BinOp::Mul, Float(x), Float(y)) => Float(*x * *y),
        (BinOp::Mul, Int(x), Float(y)) => Float(*x as f64 * *y),
        (BinOp::Mul, Float(x), Int(y)) => Float(*x * *y as f64),
        (BinOp::Lt, Int(x), Int(y)) => Value::Bool(*x < *y),
        (BinOp::Lt, Float(x), Float(y)) => Value::Bool(*x < *y),
        (BinOp::Gt, Int(x), Int(y)) => Value::Bool(*x > *y),
        (BinOp::Gt, Float(x), Float(y)) => Value::Bool(*x > *y),
        (BinOp::LtEq, Int(x), Int(y)) => Value::Bool(*x <= *y),
        (BinOp::LtEq, Float(x), Float(y)) => Value::Bool(*x <= *y),
        (BinOp::GtEq, Int(x), Int(y)) => Value::Bool(*x >= *y),
        (BinOp::GtEq, Float(x), Float(y)) => Value::Bool(*x >= *y),
        // int/float 混在の順序比較（#16 段階(b)(iii)）。`apply_binop` の混在アームと同一意味論。
        // 以前はここが無く `apply_binop_dyn` → 巨大 match まで降りていた。
        // `LtEq`/`GtEq` は #18 で `apply_binop` 側にアームが入ったので、ここでも高速パスに載せる。
        // Eq/NotEq は `apply_binop` が `values_eq` の catch-all で処理するため特化しない。
        (BinOp::Lt, Int(x), Float(y)) => Value::Bool((*x as f64) < *y),
        (BinOp::Lt, Float(x), Int(y)) => Value::Bool(*x < *y as f64),
        (BinOp::Gt, Int(x), Float(y)) => Value::Bool((*x as f64) > *y),
        (BinOp::Gt, Float(x), Int(y)) => Value::Bool(*x > *y as f64),
        (BinOp::LtEq, Int(x), Float(y)) => Value::Bool((*x as f64) <= *y),
        (BinOp::LtEq, Float(x), Int(y)) => Value::Bool(*x <= *y as f64),
        (BinOp::GtEq, Int(x), Float(y)) => Value::Bool((*x as f64) >= *y),
        (BinOp::GtEq, Float(x), Int(y)) => Value::Bool(*x >= *y as f64),
        // int/float 混在の除算・冪（ゼロ検査不要なアームのみ）。
        (BinOp::Div, Int(x), Float(y)) => Float(*x as f64 / *y),
        (BinOp::Div, Float(x), Int(y)) => Float(*x / *y as f64),
        (BinOp::Div, Float(x), Float(y)) => Float(*x / *y),
        (BinOp::Pow, Float(x), Float(y)) => Float(x.powf(*y)),
        (BinOp::Pow, Int(x), Float(y)) => Float((*x as f64).powf(*y)),
        (BinOp::Pow, Float(x), Int(y)) => Float(x.powi(*y as i32)),
        // その他の型・ゼロ除算しうる int 演算はフルパスへ。
        _ => return interp.apply_binop_dyn(op, a, b),
    };
    Ok(r)
}

/// `Value::Bool` のみ直接判定し（比較演算子の結果＝ホットな条件）、それ以外は
/// セマンティクス一致のため既存 `eval_truthy` へ委譲する。
#[inline]
pub(super) fn truthy_fast(interp: &mut Interpreter, v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => interp.eval_truthy(v),
    }
}

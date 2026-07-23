// vm/run.rs — バイトコード VM のディスパッチループ（Phase V, V-A）。
//
// 値スタックは呼び出し側が持つ単一バッファ（`buf`）を共有し、per-call 確保を避ける。
// フレームローカルは `buf[base .. base+n_locals]`、オペランドスタックはその上（`buf[base+n_locals ..]`）。
//
// 算術・比較は int/float の高速パスを VM ループ内にインライン展開し、それ以外
// （文字列・インスタンス演算子・混在型・ゼロ除算など）は既存の `apply_binop_dyn` へ委譲する。
// 高速パスは `apply_binop` の該当アームと**同一のセマンティクス**（`a + b` 等、オーバーフロー
// 挙動も含め）で書く。属性読み・真偽判定・単項も既存実装へ委譲するので結果はツリーウォークと一致する。

use crate::ast::BinOp;
use crate::interpreter::{Interpreter, Value};

use super::chunk::Chunk;
use super::op::Op;

/// Chunk を実行して戻り値を返す。
/// `buf` の `base..base+n_locals` にパラメータが束縛済み。実行後 `buf` は base+n_locals..（オペランド）
/// を空にして返る（呼び出し側が `truncate(base)` する）。
pub fn run(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
) -> Result<Value, String> {
    let code = &chunk.code;
    let mut ip: usize = 0;

    loop {
        match &code[ip] {
            Op::Const(i) => buf.push(chunk.consts[*i as usize].clone()),
            Op::Nil => buf.push(Value::None),
            Op::LoadLocal(s) => {
                let v = buf[base + *s as usize].clone();
                buf.push(v);
            }
            Op::StoreLocal(s) => {
                let v = buf.pop().unwrap();
                buf[base + *s as usize] = v;
            }
            Op::Pop => {
                buf.pop();
            }
            Op::Bin(op) => {
                let b = buf.pop().unwrap();
                let a = buf.pop().unwrap();
                let r = apply_bin_fast(interp, op, a, b)?;
                buf.push(r);
            }
            Op::Un(op) => {
                let a = buf.pop().unwrap();
                let r = interp.apply_unary_dyn(op, a)?;
                buf.push(r);
            }
            Op::GetAttr(name_idx, cache_idx) => {
                let obj = buf.pop().unwrap();
                let cache = &chunk.attr_caches[*cache_idx as usize];
                // R3 インラインキャッシュのヒット（public フィールド）を VM ループ内で直接処理し、
                // get_attr_val の関数呼び出しを避ける。ミス・非 public・非インスタンスはフルパスへ。
                let v = 'get: {
                    if let Value::Instance(inst_rc) = &obj {
                        let class_id = inst_rc.borrow().class.class_id;
                        if let Some((idx, access)) = cache.get(class_id) {
                            if access == crate::ast::AttrCache::PUBLIC {
                                let inst = inst_rc.borrow();
                                debug_assert_eq!(
                                    inst.class.field_index.get(&chunk.names[*name_idx as usize]).copied(),
                                    Some(idx),
                                    "VM GetAttr cache slot mismatch"
                                );
                                if let Some(fv) = inst.field_value(idx) {
                                    break 'get fv;
                                }
                            }
                        }
                    }
                    interp.get_attr_val(obj, &chunk.names[*name_idx as usize], Some(cache))?
                };
                buf.push(v);
            }
            Op::Jump(t) => {
                ip = *t as usize;
                continue;
            }
            Op::JumpIfFalse(t) => {
                let c = buf.pop().unwrap();
                if !truthy_fast(interp, &c)? {
                    ip = *t as usize;
                    continue;
                }
            }
            Op::JumpIfFalseOrPop(t) => {
                let truthy = truthy_fast(interp, buf.last().unwrap())?;
                if !truthy {
                    ip = *t as usize;
                    continue;
                }
                buf.pop();
            }
            Op::JumpIfTrueOrPop(t) => {
                let truthy = truthy_fast(interp, buf.last().unwrap())?;
                if truthy {
                    ip = *t as usize;
                    continue;
                }
                buf.pop();
            }
            Op::Return => return Ok(buf.pop().unwrap()),
            Op::ReturnNil => return Ok(Value::None),
        }
        ip += 1;
    }
}

/// int/float の算術・順序比較を高速パスで処理し、それ以外は `apply_binop_dyn` へ委譲する。
/// 高速パスは `apply_binop` の該当アームと同一セマンティクス。
#[inline]
fn apply_bin_fast(
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
        // 混在比較・その他の型・ゼロ除算あり演算子はフルパスへ。
        _ => return interp.apply_binop_dyn(op, a, b),
    };
    Ok(r)
}

/// `Value::Bool` のみ直接判定し（比較演算子の結果＝ホットな条件）、それ以外は
/// セマンティクス一致のため既存 `eval_truthy` へ委譲する。
#[inline]
fn truthy_fast(interp: &mut Interpreter, v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => interp.eval_truthy(v),
    }
}

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
            Op::LoadGlobal(ni) => {
                let name = &chunk.names[*ni as usize];
                match interp.vm_get_global(name) {
                    Some(v) => buf.push(v),
                    None => return Err(format!("NameError: '{name}' is not defined")),
                }
            }
            Op::StoreLocal(s) => {
                let v = buf.pop().unwrap();
                buf[base + *s as usize] = v;
            }
            Op::StoreLocalDeepCopy(s) => {
                let v = Interpreter::deep_copy_value(buf.pop().unwrap());
                buf[base + *s as usize] = v;
            }
            Op::StoreLocalCopyFreeze(s) => {
                let v = Interpreter::deep_copy_value(buf.pop().unwrap());
                interp.apply_freeze_to_value(&v, true)?;
                buf[base + *s as usize] = v;
            }
            Op::StoreLocalFreezeInstance(s) => {
                let v = buf.pop().unwrap();
                let v = if matches!(v, Value::Instance(_)) {
                    let copied = Interpreter::deep_copy_value(v);
                    interp.apply_freeze_to_value(&copied, true)?;
                    copied
                } else {
                    v
                };
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
            Op::SetAttr(name_idx) => {
                let value = buf.pop().unwrap();
                let obj = buf.pop().unwrap();
                interp.attr_assign_evaled(obj, &chunk.names[*name_idx as usize], value)?;
            }
            Op::Swap => {
                let n = buf.len();
                buf.swap(n - 1, n - 2);
            }
            Op::IsType(name_idx) => {
                let v = buf.pop().unwrap();
                let r = interp.value_is_type(&v, &chunk.names[*name_idx as usize]);
                buf.push(Value::Bool(r));
            }
            Op::CallBuiltin(name_idx, argc) => {
                let n = *argc as usize;
                let split = buf.len() - n;
                let args = buf.split_off(split); // arg0..argN-1（順序保持）
                let name = &chunk.names[*name_idx as usize];
                match interp.eval_builtin_evaled(name, args) {
                    Some(r) => buf.push(r?),
                    // コンパイラは eval_builtin_evaled が扱う名前だけ発行するので到達しない。
                    None => return Err(format!("NameError: '{name}' is not defined")),
                }
            }
            Op::GetIter => {
                let iterable = buf.pop().unwrap();
                let iter = interp.make_for_iterator(iterable)?;
                buf.push(iter);
            }
            Op::ForIter(iter_slot, target_slot, exit_ip) => {
                let iter_idx = base + *iter_slot as usize;
                // 高速パス: Generator（range/list/str/set/tuple/gen __iter__ の実体）は
                // index を直接進める（eval_method_call のディスパッチを丸ごと回避）。
                // メソッド呼び出しの Generator "next" アームと同一意味論。
                let next: Option<Option<Value>> =
                    if let Value::Generator(state) = &buf[iter_idx] {
                        let mut s = state.borrow_mut();
                        if s.index < s.values.len() {
                            let val = s.values[s.index].clone();
                            s.index += 1;
                            Some(Some(val))
                        } else {
                            Some(None) // 枯渇
                        }
                    } else {
                        None // 非 Generator（カスタムイテレータ）はフォールバック
                    };
                match next {
                    Some(Some(val)) => buf[base + *target_slot as usize] = val,
                    Some(None) => {
                        ip = *exit_ip as usize;
                        continue;
                    }
                    None => {
                        // フォールバック: カスタムイテレータ等は .next() を呼ぶ。
                        let iter = buf[iter_idx].clone();
                        match interp.eval_method_call(iter, "next", &[], None) {
                            Ok(item) => buf[base + *target_slot as usize] = item,
                            Err(ref e) if e.starts_with("EndOfIteration") => {
                                ip = *exit_ip as usize;
                                continue;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
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
            Op::Call(argc, mut_mask) => {
                let n = *argc as usize;
                let split = buf.len() - n;
                let arg_vals = buf.split_off(split); // arg0..argN-1（順序保持）
                let callee = buf.pop().unwrap();
                let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                    .collect();
                let r = interp.call_value_evaled(callee, evaled)?;
                buf.push(r);
            }
            Op::CallMethod(name_idx, argc, mut_mask) => {
                let n = *argc as usize;
                let split = buf.len() - n;
                let arg_vals = buf.split_off(split);
                let obj = buf.pop().unwrap();
                let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                    .collect();
                let r = interp.call_instance_method_evaled(
                    obj,
                    &chunk.names[*name_idx as usize],
                    evaled,
                    Some(&chunk.attr_caches[*name_idx as usize]),
                )?;
                buf.push(r);
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

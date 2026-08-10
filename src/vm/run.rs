// vm/run.rs — バイトコード VM のディスパッチループ（Phase V, V-A）。
//
// 値スタックは呼び出し側が持つ単一バッファ（`buf`）を共有し、per-call 確保を避ける。
// フレームローカルは `buf[base .. base+n_locals]`、オペランドスタックはその上（`buf[base+n_locals ..]`）。
//
// 算術・比較は int/float の高速パスを VM ループ内にインライン展開し、それ以外
// （文字列・インスタンス演算子・混在型・ゼロ除算など）は既存の `apply_binop_dyn` へ委譲する。
// 高速パスは `apply_binop` の該当アームと**同一のセマンティクス**（`a + b` 等、オーバーフロー
// 挙動も含め）で書く。属性読み・真偽判定・単項も既存実装へ委譲するので結果はツリーウォークと一致する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::BinOp;
use crate::interpreter::{Interpreter, Value};

use super::chunk::Chunk;
use super::op::Op;

/// 1 命令実行後の制御フロー。
enum Flow {
    /// 次の命令へ（ip += 1）。
    Next,
    /// 絶対 index へジャンプ。
    Jump(usize),
    /// 関数から値を返す。
    Return(Value),
}

/// アクティブな例外ハンドラ（try 節ごとに VM のハンドラスタックに積む）。
struct Handler {
    /// 例外発生時に飛ぶ landing pad の ip。
    handler_ip: usize,
    /// try 進入時のオペランドスタック深さ（例外時にここまで巻き戻す）。
    stack_len: usize,
}

/// Chunk を実行して戻り値を返す。
/// `buf` の `base..base+n_locals` にパラメータが束縛済み。実行後 `buf` は base+n_locals..（オペランド）
/// を空にして返る（呼び出し側が `truncate(base)` する）。
pub fn run(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
) -> Result<Value, String> {
    let mut ip: usize = 0;
    let mut handlers: Vec<Handler> = Vec::new();

    loop {
        match exec_op(interp, chunk, buf, base, ip, &mut handlers) {
            Ok(Flow::Next) => ip += 1,
            Ok(Flow::Jump(t)) => ip = t,
            Ok(Flow::Return(v)) => return Ok(v),
            Err(e) => {
                // 例外: 最内ハンドラがあればオペランドを巻き戻して例外値を積み landing pad へ。
                // 変換できない（active exception なし等）・ハンドラなしなら伝播（Err を返す）。
                match handlers.pop() {
                    Some(h) => {
                        buf.truncate(h.stack_len);
                        match interp.vm_take_raised(&e) {
                            Some(exc_val) => {
                                buf.push(exc_val);
                                ip = h.handler_ip;
                            }
                            None => return Err(e),
                        }
                    }
                    None => return Err(e),
                }
            }
        }
    }
}

/// 単一命令を実行し、制御フローを返す。エラーは `Err` で返し、`run` がハンドラスタックへ回す。
#[inline]
fn exec_op(
    interp: &mut Interpreter,
    chunk: &Chunk,
    buf: &mut Vec<Value>,
    base: usize,
    ip: usize,
    handlers: &mut Vec<Handler>,
) -> Result<Flow, String> {
    match &chunk.code[ip] {
        Op::Const(i) => buf.push(chunk.consts[*i as usize].clone()),
        Op::Nil => buf.push(Value::None),
        Op::LoadLocal(s) => {
            let v = buf[base + *s as usize].clone();
            buf.push(v);
        }
        Op::LoadGlobal(ni, ci) => {
            // #11: グローバル索引キャッシュ。ヒット時は名前ハッシュ引きを飛ばし slot 直読み。
            let cache = &chunk.global_caches[*ci as usize];
            let epoch = interp.vm_slot_epoch();
            if let Some(idx) = cache.get(epoch) {
                if let Some(v) = interp.vm_global_by_slot(idx) {
                    buf.push(v);
                    return Ok(Flow::Next);
                }
                // 想定外（index 失効）は名前引きへフォールバック。
            }
            let name = &chunk.names[*ni as usize];
            match interp.vm_global_slot_of(name) {
                Some(idx) => {
                    cache.fill(epoch, idx as u32);
                    // 直前に解決した index からそのまま読む（None は理論上起きない）。
                    match interp.vm_global_by_slot(idx) {
                        Some(v) => buf.push(v),
                        None => return Err(format!("NameError: '{name}' is not defined")),
                    }
                }
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
        // 超命令（#2）: オペランドを frame/const から直接読み、push/pop とディスパッチを省く。
        Op::BinLocalLocal(a, b, op) => {
            let lhs = buf[base + *a as usize].clone();
            let rhs = buf[base + *b as usize].clone();
            let r = apply_bin_fast(interp, op, lhs, rhs)?;
            buf.push(r);
        }
        Op::BinLocalConst(a, ci, op) => {
            let lhs = buf[base + *a as usize].clone();
            let rhs = chunk.consts[*ci as usize].clone();
            let r = apply_bin_fast(interp, op, lhs, rhs)?;
            buf.push(r);
        }
        // 型特化（plan A）: オペランドを参照で読み、int/float 直接算術。想定外型は汎用へ委譲。
        Op::IntBinLL(a, b, op) => {
            let r = match (&buf[base + *a as usize], &buf[base + *b as usize]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = buf[base + *b as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        Op::IntBinLC(a, ci, op) => {
            let r = match (&buf[base + *a as usize], &chunk.consts[*ci as usize]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = chunk.consts[*ci as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinLL(a, b, op) => {
            let r = match (&buf[base + *a as usize], &buf[base + *b as usize]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = buf[base + *b as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinLC(a, ci, op) => {
            let r = match (&buf[base + *a as usize], &chunk.consts[*ci as usize]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => buf.push(v),
                None => {
                    let l = buf[base + *a as usize].clone();
                    let rr = chunk.consts[*ci as usize].clone();
                    let v = apply_bin_fast(interp, op, l, rr)?;
                    buf.push(v);
                }
            }
        }
        // 型特化（#16 段階(b)(iii)）: オペランドの形を問わずスタック上の2値を参照で見る。
        // 属性・添字・呼び出し結果など `LL`/`LC` では拾えなかった式にも特化が乗る。
        Op::IntBinSS(op) => {
            let n = buf.len();
            let r = match (&buf[n - 2], &buf[n - 1]) {
                (Value::Int(x), Value::Int(y)) => int_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => {
                    buf.truncate(n - 2);
                    buf.push(v);
                }
                None => {
                    let b = buf.pop().unwrap();
                    let a = buf.pop().unwrap();
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
        }
        Op::FloatBinSS(op) => {
            let n = buf.len();
            let r = match (&buf[n - 2], &buf[n - 1]) {
                (Value::Float(x), Value::Float(y)) => float_binop_specialized(*x, *y, op),
                _ => None,
            };
            match r {
                Some(v) => {
                    buf.truncate(n - 2);
                    buf.push(v);
                }
                None => {
                    let b = buf.pop().unwrap();
                    let a = buf.pop().unwrap();
                    let v = apply_bin_fast(interp, op, a, b)?;
                    buf.push(v);
                }
            }
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
        // 超命令（#16 段階(b)(i)）: レシーバを frame から**参照で**読む。`GetAttr` と同一意味論だが
        // `LoadLocal` の `Value` clone（＝`Rc` refcount 増減）と push/pop が消える。
        // IC ミス・非 public・非インスタンスのときだけ clone してフルパスへ回す。
        Op::GetAttrLocal(slot, name_idx, cache_idx) => {
            let cache = &chunk.attr_caches[*cache_idx as usize];
            let v = 'get: {
                if let Value::Instance(inst_rc) = &buf[base + *slot as usize] {
                    let inst = inst_rc.borrow();
                    if let Some((idx, access)) = cache.get(inst.class.class_id) {
                        if access == crate::ast::AttrCache::PUBLIC {
                            debug_assert_eq!(
                                inst.class.field_index.get(&chunk.names[*name_idx as usize]).copied(),
                                Some(idx),
                                "VM GetAttrLocal cache slot mismatch"
                            );
                            if let Some(fv) = inst.field_value(idx) {
                                break 'get fv;
                            }
                        }
                    }
                }
                let obj = buf[base + *slot as usize].clone();
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
        // `CheckBefore` 指示の消費（#16 段階(b)(ii)）。
        // 検査本体はツリーウォークと**同一メソッド**（`mustbe_check`）を呼ぶので意味論が構造的に一致する。
        Op::MustBe(type_idx, span_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.mustbe_check(
                v,
                &chunk.names[*type_idx as usize],
                &chunk.spans[*span_idx as usize],
            )?;
            buf.push(r);
        }
        Op::Cast(type_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.eval_cast_evaled(v, &chunk.names[*type_idx as usize])?;
            buf.push(r);
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
                Some(None) => return Ok(Flow::Jump(*exit_ip as usize)),
                None => {
                    // フォールバック: カスタムイテレータ等は .next() を呼ぶ。
                    let iter = buf[iter_idx].clone();
                    match interp.eval_method_call(iter, "next", &[], None) {
                        Ok(item) => buf[base + *target_slot as usize] = item,
                        Err(ref e) if e.starts_with("EndOfIteration") => {
                            return Ok(Flow::Jump(*exit_ip as usize))
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Op::Jump(t) => return Ok(Flow::Jump(*t as usize)),
        Op::JumpIfFalse(t) => {
            let c = buf.pop().unwrap();
            if !truthy_fast(interp, &c)? {
                return Ok(Flow::Jump(*t as usize));
            }
        }
        Op::JumpIfFalseOrPop(t) => {
            let truthy = truthy_fast(interp, buf.last().unwrap())?;
            if !truthy {
                return Ok(Flow::Jump(*t as usize));
            }
            buf.pop();
        }
        Op::JumpIfTrueOrPop(t) => {
            let truthy = truthy_fast(interp, buf.last().unwrap())?;
            if truthy {
                return Ok(Flow::Jump(*t as usize));
            }
            buf.pop();
        }
        Op::Call(argc, mut_mask, name_idx, span_idx) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split); // arg0..argN-1（順序保持）
            let callee = buf.pop().unwrap();
            let evaled: Vec<(Option<String>, Value, bool)> = arg_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (None, v, (mut_mask >> i) & 1 == 1))
                .collect();
            // 呼び出し元名・位置をトレースバック用に渡す（V-E）。
            let r = interp.call_value_evaled(
                callee,
                evaled,
                &chunk.names[*name_idx as usize],
                Some(chunk.spans[*span_idx as usize].clone()),
            )?;
            buf.push(r);
        }
        // 超命令（#16 段階(b)(i)）: レシーバを frame から直接取る。`CallMethod` と同一意味論。
        Op::CallMethodLocal(slot, name_idx, argc, mut_mask) => {
            let n = *argc as usize;
            let split = buf.len() - n;
            let arg_vals = buf.split_off(split);
            // レシーバは呼び先の `self` として所有権が要るので clone は避けられないが、
            // `LoadLocal` の op ディスパッチと push/pop は省ける。
            let obj = buf[base + *slot as usize].clone();
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
                None,
            )?;
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
            // メソッドはツリーウォークと同じく call_span=None（degraded フレームで一致）。
            let r = interp.call_instance_method_evaled(
                obj,
                &chunk.names[*name_idx as usize],
                evaled,
                Some(&chunk.attr_caches[*name_idx as usize]),
                None,
            )?;
            buf.push(r);
        }
        Op::Return => return Ok(Flow::Return(buf.pop().unwrap())),
        Op::ReturnNil => return Ok(Flow::Return(Value::None)),
        // ── 例外処理（Phase V-C） ──
        Op::SetupTry(handler_ip) => {
            handlers.push(Handler {
                handler_ip: *handler_ip as usize,
                stack_len: buf.len(),
            });
        }
        Op::PopTry => {
            handlers.pop();
        }
        Op::Raise(span_idx) => {
            let exc = buf.pop().unwrap();
            let sentinel = interp.vm_raise(exc, &chunk.spans[*span_idx as usize]);
            return Err(sentinel);
        }
        Op::Reraise => {
            let sentinel = interp.vm_reraise();
            return Err(sentinel);
        }
        Op::Dup => {
            let v = buf.last().unwrap().clone();
            buf.push(v);
        }
        Op::ExcMatch(name_idx) => {
            let v = buf.pop().unwrap();
            let r = interp.vm_exc_matches(&v, &chunk.names[*name_idx as usize]);
            buf.push(Value::Bool(r));
        }
        Op::BuildEmptyList => buf.push(Value::List(Rc::new(RefCell::new(Vec::new())))),
        Op::ListAppendLocal(slot) => {
            let v = buf.pop().unwrap();
            if let Value::List(list) = &buf[base + *slot as usize] {
                list.borrow_mut().push(v);
            }
        }
        Op::ListOrNone => {
            let list = buf.pop().unwrap();
            let empty = matches!(&list, Value::List(l) if l.borrow().is_empty());
            buf.push(if empty { Value::None } else { list });
        }
        Op::LoadName(name_idx) => {
            let name = &chunk.names[*name_idx as usize];
            match interp.vm_load_name(name) {
                Some(v) => buf.push(v),
                None => return Err(format!("NameError: '{name}' is not defined")),
            }
        }
        Op::DeclareName(name_idx) => {
            let v = buf.pop().unwrap();
            interp.vm_declare_debug(&chunk.names[*name_idx as usize], v)?;
        }
        Op::Subscript => {
            let key = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            let v = interp.eval_subscript(obj, key)?;
            buf.push(v);
        }
        Op::SetIndex => {
            let value = buf.pop().unwrap();
            let key = buf.pop().unwrap();
            let obj = buf.pop().unwrap();
            interp.eval_setitem(obj, key, value)?;
        }
        Op::BuildList(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_list(vals));
        }
        Op::BuildTuple(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_tuple(vals));
        }
        Op::BuildSet(n) => {
            let split = buf.len() - *n as usize;
            let vals = buf.split_off(split);
            buf.push(interp.vm_build_set(vals));
        }
        Op::BuildDict(n) => {
            let split = buf.len() - 2 * *n as usize;
            let flat = buf.split_off(split);
            buf.push(interp.vm_build_dict(flat));
        }
        Op::Yield => {
            let v = buf.pop().unwrap();
            interp.vm_yield_push(v);
        }
        Op::AsyncSubmit(idx) => {
            let mgr = buf.pop().unwrap();
            let block = &chunk.async_blocks[*idx as usize];
            // 捕捉変数を frame の slot から読み出す（capture_env と同じ mutable/immutable 規則は
            // vm_async_submit 内で Var 経由に適用する）。
            let captured: Vec<(String, Value, bool)> = block
                .captures
                .iter()
                .map(|(name, slot, is_mut)| (name.clone(), buf[base + *slot as usize].clone(), *is_mut))
                .collect();
            interp.vm_async_submit(mgr, &block.body, captured)?;
        }
    }
    Ok(Flow::Next)
}

/// int 型特化二項演算（plan A）。`apply_bin_fast` の (Int,Int) アームと同一意味論。
/// Add/Sub/Mul と比較のみ対応（Div/Mod/Pow/bit・ゼロ除算しうる op は `None`＝汎用へ委譲）。
#[inline]
fn int_binop_specialized(x: i64, y: i64, op: &BinOp) -> Option<Value> {
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
            Value::Int(x.div_euclid(y))
        }
        BinOp::Mod => {
            if y == 0 {
                return None;
            }
            Value::Int(x.rem_euclid(y))
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
fn float_binop_specialized(x: f64, y: f64, op: &BinOp) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Float(x + y),
        BinOp::Sub => Value::Float(x - y),
        BinOp::Mul => Value::Float(x * y),
        // float の除算はゼロ検査なし（inf/NaN を返す）＝ `apply_binop` の Float/Float アームと同一。
        // `//` と `%` は Float/Float のアームが存在しない（＝エラー）ため特化しない。
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
        // int/float 混在の順序比較（#16 段階(b)(iii)）。`apply_binop` の混在アームと同一意味論。
        // 以前はここが無く `apply_binop_dyn` → 巨大 match まで降りていた。
        // **`LtEq`/`GtEq` の混在アームは `apply_binop` に存在しない**（`i < f` は通るが `i <= f` は
        // TypeError になるのが現状の言語仕様）。ここで足すと意味論が変わるので追加しない。
        // Eq/NotEq は `apply_binop` が `values_eq` の catch-all で処理するため特化しない。
        (BinOp::Lt, Int(x), Float(y)) => Value::Bool((*x as f64) < *y),
        (BinOp::Lt, Float(x), Int(y)) => Value::Bool(*x < *y as f64),
        (BinOp::Gt, Int(x), Float(y)) => Value::Bool((*x as f64) > *y),
        (BinOp::Gt, Float(x), Int(y)) => Value::Bool(*x > *y as f64),
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
fn truthy_fast(interp: &mut Interpreter, v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => interp.eval_truthy(v),
    }
}

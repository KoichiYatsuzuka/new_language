// native_api/callbacks.rs — ネイティブ DLL へ渡す C ABI コールバック(`extern "C" fn ar_*`)群と
// それらを束ねる `CALLBACKS` テーブル。全ての重い操作はインタープリタ経由でルーティングする。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{DictData, TupleData, Value},
};
use super::*;

extern "C" fn ar_make_int(n: i64) -> i64 {
    push_handle(Value::Int(n))
}

extern "C" fn ar_make_float(f: f64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.push_value(Value::Float(f))
    })
}

extern "C" fn ar_make_bool(b: i32) -> i64 {
    if b != 0 { TL_TRUE } else { TL_FALSE }
}

extern "C" fn ar_make_str(ptr: *const u8, len: i32) -> i64 {
    let text = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize))
    }
    .to_owned();
    STATE.with(|s| s.borrow_mut().push_value(Value::str(text)))
}

// Build list/tuple/dict in one borrow: clone all items then push the container.
extern "C" fn ar_make_list(items_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let items: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let list = Rc::new(RefCell::new(items));
        st.push_value(Value::List(list))
    })
}

extern "C" fn ar_make_tuple(items_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let elements: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let tuple = Rc::new(TupleData::new(elements, vec![]));
        st.push_value(Value::Tuple(tuple))
    })
}

extern "C" fn ar_make_dict(keys_ptr: *const i64, vals_ptr: *const i64, n: i32) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let mut dict = DictData::new("Any".to_string(), "Any".to_string());
        for i in 0..n as usize {
            let k = st.clone_value(unsafe { *keys_ptr.add(i) });
            let v = st.clone_value(unsafe { *vals_ptr.add(i) });
            dict.set(k, v);
        }
        st.push_value(Value::Dict(Rc::new(RefCell::new(dict))))
    })
}

extern "C" fn ar_make_none() -> i64 {
    TL_NONE
}

extern "C" fn ar_is_truthy(h: i64) -> i32 {
    // Phase 1: extract value and interp_ptr.
    let (v, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.clone_value(h), st.interp_ptr)
    });
    if interp_ptr.is_null() {
        return match &v {
            Value::None => 0,
            Value::Bool(b) => i32::from(*b),
            Value::Int(n) => i32::from(*n != 0),
            Value::Float(f) => i32::from(*f != 0.0),
            Value::Str(s) => i32::from(!s.is_empty()),
            _ => 1,
        };
    }
    let interp = unsafe { &mut *interp_ptr };
    i32::from(interp.is_truthy(&v))
}

extern "C" fn ar_binop(op: i32, a: i64, b: i64) -> i64 {
    let (has_err, lhs, rhs, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(a), st.clone_value(b), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let ast_op = match i32_to_binop(op) {
        Some(o) => o,
        None => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: invalid binop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for binop".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &*interp_ptr };
    match interp.apply_binop(&ast_op, lhs, rhs) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_unop(op: i32, a: i64) -> i64 {
    let (has_err, operand, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(a), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    use crate::ast::UnaryOp;
    let ast_op = match op {
        UOP_NEG => UnaryOp::Neg,
        UOP_NOT => UnaryOp::Not,
        UOP_BIT_NOT => UnaryOp::BitNot,
        _ => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: invalid unop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for unop".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &*interp_ptr };
    match interp.apply_unary(&ast_op, operand) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 関数ハンドルと引数ハンドル配列を受け取り関数を呼び出す C コールバック。
///
/// Fast path: `Value::NativeFunction` はインタープリタを経由せず直接 DLL 関数を呼び出す。
/// Slow path: クロージャ・ユーザー定義関数などは `call_value_with_args` に委譲する。
extern "C" fn ar_call_fn(fn_h: i64, args_ptr: *const i64, n_args: i32) -> i64 {
    use std::sync::atomic::Ordering;

    // Phase 1: extract fn_val and interp_ptr, check for pending error.
    let (has_err, fn_val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(fn_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // ── 統一 typed ABI 最速パス（ネイティブ→C DLL） ─────────────────────────
    // 引数ハンドルを1回の STATE ボローで u64 スロットに展開し、{name}_typed を
    // 直接呼ぶ。enter/exit_native_call・per-arg unmarshal コールバック・
    // 結果 marshal コールバックがすべて消える（STATE アクセス ~8回 → 1-2回）。
    // 構造体ポインタ引数（AbiTy::Ptr）を含む場合はゼロコピー／シャドウ変換を解決する
    // 別経路を通る（P3/P4 — .claude/skills/c-abi-interop/SKILL.md）。この層は named-variable の
    // 概念がない（アリーナハンドル止まり）ため let/mut 判定は常に `None` で渡す。
    if let Value::NativeFunction(ref fn_ref) = fn_val {
        if let Some(sig) = &fn_ref.typed_sig {
            let typed_ptr = fn_ref.typed_fn_ptr.load(Ordering::Relaxed);
            let n = n_args as usize;
            if typed_ptr != 0 && n == sig.params.len() && n <= 16 {
                use crate::interpreter::value::{AbiTy, Marshalled, TypedArgs};
                let has_ptr = sig
                    .params
                    .iter()
                    .any(|p| matches!(p, AbiTy::Ptr { .. } | AbiTy::OutPtr { .. }));
                // #76: 引数マーシャリングは `TypedArgs` に 1 本化した。
                // ⚠⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
                let mut ta = TypedArgs::new();
                let mut call_err: Option<String> = None;
                let ok = if !has_ptr {
                    // 純スカラー: 1回の STATE ボローでデコード（Value クローンなし）。
                    // ⚠ **この枝だけ共有マーシャラを通さない**（#76 で残した唯一の例外）:
                    // ハンドル → Value のクローンを挟まずアリーナから直接デコードする最適化で、
                    // STATE アクセスが ~8 回 → 1〜2 回になる。型の許容表は下の枝と同一。
                    STATE.with(|s| {
                        let st = s.borrow();
                        for i in 0..n {
                            let h = unsafe { *args_ptr.add(i) };
                            let (iv, fv): (Option<i64>, Option<f64>) = match h {
                                x if (3..INT_CACHE_BASE as i64).contains(&x) => (Some(x - 3), None),
                                x if x >= INT_CACHE_BASE as i64 => {
                                    match st.arena.get(x as usize) {
                                        Some(Value::Int(v)) => (Some(*v), None),
                                        Some(Value::Float(f)) => (None, Some(*f)),
                                        _ => return false,
                                    }
                                }
                                _ => return false, // None/Bool/sentinel → ハンドル経路へ
                            };
                            ta.slots[i] = match (iv, fv, &sig.params[i]) {
                                (Some(v), _, AbiTy::I64) => v as u64,
                                (Some(v), _, AbiTy::F64) => (v as f64).to_bits(),
                                (_, Some(f), AbiTy::F64) => f.to_bits(),
                                _ => return false,
                            };
                        }
                        true
                    })
                } else {
                    // 構造体ポインタあり: 1回の STATE ボローでハンドルを Value に解決
                    // （Instance は Rc クローンのみで安価）、以降は STATE の外で処理する。
                    let vals: Vec<Value> = STATE.with(|s| {
                        let st = s.borrow();
                        (0..n).map(|i| st.clone_value(unsafe { *args_ptr.add(i) })).collect()
                    });
                    // ⚠ この層は named-variable の概念が無い（アリーナハンドル止まり）ので
                    // `named_mut` は**全部 None**＝書き戻ししない（安全側）。#76 で畳んだ結果、
                    // 4 経路で違うのは**この入力だけ**になった。
                    match ta.marshal(&vals, &sig.params, &[]) {
                        Ok(Marshalled::Ready) => true,
                        Ok(Marshalled::TypeMismatch) => false,
                        Err(e) => {
                            call_err = Some(e);
                            false
                        }
                    }
                };
                if let Some(e) = call_err {
                    STATE.with(|s| s.borrow_mut().error = Some(e));
                    return TL_NONE;
                }
                if ok {
                    // #76: 呼び出し本体も `invoke_typed_abi` に合流させた（cleanup の実行順と
                    // status の扱いが 4 経路で同一になる）。⚠ この層はエラーを戻り値で返せない
                    // ので `STATE.error` に載せて TL_NONE を返す — 違いはそこだけ。
                    // SAFETY: `typed_ptr` は typed_sig 付き（`build_cpp_typed_sig` 検証済み）の関数。
                    let ret = match unsafe {
                        crate::interpreter::value::invoke_typed_abi(
                            typed_ptr,
                            &ta.slots,
                            std::mem::take(&mut ta.cleanups),
                        )
                    } {
                        Ok(r) => r,
                        Err(e) => {
                            STATE.with(|s| s.borrow_mut().error = Some(e));
                            return TL_NONE;
                        }
                    };
                    // ⚠ OutPtr の書き戻しは無い（名前が無いので安全側 ＝ `ta.out_wb` は常に空）。
                    return match sig.ret {
                        // 小整数はキャッシュ済みハンドル（STATE アクセスなし）
                        AbiTy::I64 => push_handle(Value::Int(ret as i64)),
                        AbiTy::F64 => push_handle(Value::Float(f64::from_bits(ret))),
                        AbiTy::Void => TL_NONE,
                        AbiTy::Ptr { .. } | AbiTy::OutPtr { .. } => {
                            unreachable!("typed ABI ret excludes Ptr/OutPtr")
                        }
                    };
                }
                // 型不一致 → 既存のハンドル経路にフォールバック
            }
        }
    }

    // Fast path: NativeFunction (native DLL).
    // Arg handles are already live in the arena — pass them straight through.
    if let Value::NativeFunction(ref fn_ref) = fn_val {
        if !interp_ptr.is_null() {
            let is_outermost = enter_native_call(interp_ptr);

            // DLL: use cached_fn_ptr; resolve on first call.
            let cached = fn_ref.cached_fn_ptr.load(Ordering::Relaxed);
            let fn_ptr = if cached != 0 {
                cached
            } else {
                let sym_name = format!("{}_tl\0", fn_ref.fn_name);
                let interp = unsafe { &*interp_ptr };
                match interp.native_libs.get(&fn_ref.lib_path) {
                    Some(lib) => {
                        match unsafe {
                            lib.0.get::<unsafe extern "C" fn(*const i64, i32) -> i64>(
                                sym_name.as_bytes(),
                            )
                        } {
                            Ok(f) => {
                                let fp = *f as usize;
                                fn_ref.cached_fn_ptr.store(fp, Ordering::Relaxed);
                                fp
                            }
                            Err(e) => {
                                STATE.with(|s| s.borrow_mut().error = Some(format!(
                                    "RuntimeError: symbol '{}' not found: {e}",
                                    fn_ref.fn_name
                                )));
                                abort_native_call(is_outermost);
                                return TL_NONE;
                            }
                        }
                    }
                    None => {
                        STATE.with(|s| s.borrow_mut().error = Some(format!(
                            "RuntimeError: native library not loaded: {}",
                            fn_ref.lib_path.display()
                        )));
                        abort_native_call(is_outermost);
                        return TL_NONE;
                    }
                }
            };
            let result_h = unsafe {
                let f: unsafe extern "C" fn(*const i64, i32) -> i64 =
                    std::mem::transmute(fn_ptr);
                f(args_ptr, n_args)
            };

            if STATE.with(|s| s.borrow().error.is_some()) {
                abort_native_call(is_outermost);
                return TL_NONE;
            }
            let result_val = exit_native_call(result_h, is_outermost);
            return push_handle(result_val);
        }
    }

    // Slow path: HV functions, closures, overloaded fns, etc.
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(
            "NativeError: interpreter not set for call_fn".to_string()
        ));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_value_with_args(fn_val, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_attr(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for get_attr".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, &name, None) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_set_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, obj, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for set_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.set_attr_val(obj, &name, val) {
        STATE.with(|s| s.borrow_mut().error = Some(e));
    }
}

extern "C" fn ar_subscript(obj_h: i64, key_h: i64) -> i64 {
    let (has_err, obj, key, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(key_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for subscript".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.eval_subscript(obj, key) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_global(name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();
    let (has_err, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (looking up '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_val(&name) {
        Some(v) => push_handle(v),
        None => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("NameError: '{name}' is not defined")));
            TL_NONE
        }
    }
}

/// イテラブルなオブジェクトハンドルからイテレータハンドルを生成する。
/// イテレータハンドルは `-(idx+2)` の負値でエンコードされる。
extern "C" fn ar_iter_from(obj_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() { return TL_NONE; }
        let obj = st.clone_value(obj_h);
        let items: Vec<Value> = match obj {
            Value::List(l) => l.borrow().clone(),
            Value::FrozenList { ref state, ref layout } => {
                let fst = state.borrow();
                (0..fst.len).map(|i| layout.reconstruct_item(&fst.data, i)).collect()
            }
            Value::Tuple(t) => t.all_values().to_vec(),
            Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
            Value::Dict(d) => d.borrow().all_keys(),
            other => {
                st.error = Some(format!(
                    "TypeError: value of type '{}' is not iterable in native context",
                    match &other {
                        Value::Int(_) => "int",
                        Value::Float(_) => "float",
                        Value::Bool(_) => "bool",
                        Value::None => "NoneType",
                        _ => "object",
                    }
                ));
                return TL_NONE;
            }
        };
        let idx = st.iter_table.len();
        st.iter_table.push((items, 0));
        -((idx as i64) + 2)
    })
}

/// イテレータハンドルから次の要素ハンドルを取得する。
extern "C" fn ar_iter_next(iter_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() { return TL_STOP_ITER; }
        if iter_h > -2 {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
            return TL_STOP_ITER;
        }
        let idx = (-(iter_h + 2)) as usize;
        // Extract the next value from iter_table, then push to arena separately
        // to avoid simultaneous mutable access to two fields via a method call.
        let maybe_v: Option<Value> = st.iter_table.get_mut(idx).and_then(|(items, pos)| {
            if *pos < items.len() {
                let v = items[*pos].clone();
                *pos += 1;
                Some(v)
            } else {
                None
            }
        });
        match maybe_v {
            Some(v) => st.push_value(v),
            None => TL_STOP_ITER,
        }
    })
}

extern "C" fn ar_is_type(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.clone_value(obj_h), st.interp_ptr)
    });
    let result = if !interp_ptr.is_null() {
        let interp = unsafe { &*interp_ptr };
        interp.value_is_type(&obj, name)
    } else {
        match &obj {
            Value::Int(_) => name == "int",
            Value::Float(_) => name == "float",
            Value::Str(_) => name == "str",
            Value::Bool(_) => name == "bool",
            Value::None => name == "None" || name == "NoneType",
            Value::List(_) => name == "list" || name == "list_like",
            Value::FrozenList { .. } => name == "fixed_list" || name == "list_like",
            Value::Dict(_) => name == "dict",
            Value::Tuple(_) => name == "tuple",
            _ => false,
        }
    };
    if result { TL_TRUE } else { TL_FALSE }
}

// ── Arena save / compact helpers ────────────────────────────────────────────
//
// Called around every intra-module function call in generated code to prevent
// arena blowup.  Without these, each call leaves ~N×ops intermediate handles
// in the arena for the entire duration of the outermost native call.

extern "C" fn ar_arena_save() -> u64 {
    STATE.with(|s| s.borrow().arena.len() as u64)
}

extern "C" fn ar_arena_compact(h: i64, saved: u64) -> i64 {
    let saved = saved as usize;
    // Special handles (None/True/False): just truncate — no arena slot.
    if h < 3 {
        STATE.with(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    if (h as usize) < saved {
        // Handle lives before the save point — still valid after truncation.
        STATE.with(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    // Handle was allocated inside the callee: clone, truncate, re-push — all in one borrow.
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(h);
        st.arena.truncate(saved);
        st.push_value(val)
    })
}

/// ループ本体終端で複数の生存変数ハンドルをまとめて保存点まで巻き戻す。
/// 全クローン・truncate・再プッシュを1回の STATE borrow 内で完了する。
extern "C" fn ar_compact_many(handles_in: *const i64, n: i32, save: u64, handles_out: *mut i64) {
    let n = n as usize;
    let ins = unsafe { std::slice::from_raw_parts(handles_in, n) };
    let outs = unsafe { std::slice::from_raw_parts_mut(handles_out, n) };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if n == 0 {
            st.arena.truncate(save as usize);
            return;
        }
        let vals: Vec<Value> = ins.iter().map(|&h| st.clone_value(h)).collect();
        st.arena.truncate(save as usize);
        for (i, val) in vals.into_iter().enumerate() {
            outs[i] = st.push_value(val);
        }
    });
}

// ── Typed value extraction ───────────────────────────────────────────────────

extern "C" fn ar_deep_copy(h: i64) -> i64 {
    let val = clone_value_at(h);
    let copied = crate::interpreter::Interpreter::deep_copy_value(val);
    push_handle(copied)
}

extern "C" fn ar_to_int(h: i64) -> i64 {
    match h {
        TL_NONE => 0,
        TL_TRUE => 1,
        TL_FALSE => 0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => n - 3,
        n => STATE.with(|s| match s.borrow().arena.get(n as usize) {
            Some(Value::Int(v)) => *v,
            Some(Value::UInt(v)) => *v as i64,
            Some(Value::Float(f)) => *f as i64,
            Some(Value::Bool(b)) => *b as i64,
            _ => 0,
        }),
    }
}

extern "C" fn ar_to_float(h: i64) -> f64 {
    match h {
        TL_NONE => 0.0,
        TL_TRUE => 1.0,
        TL_FALSE => 0.0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => (n - 3) as f64,
        n => STATE.with(|s| match s.borrow().arena.get(n as usize) {
            Some(Value::Int(v)) => *v as f64,
            Some(Value::UInt(v)) => *v as f64,
            Some(Value::Float(f)) => *f,
            Some(Value::Bool(b)) => *b as u8 as f64,
            _ => 0.0,
        }),
    }
}

// ── cpp-bridge helpers ───────────────────────────────────────────────────────

extern "C" fn ar_to_cstr(h: i64) -> *const u8 {
    let s = match clone_value_at(h) {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    };
    STATE.with(|s_tls| {
        let mut st = s_tls.borrow_mut();
        let mut bytes = s.into_bytes();
        bytes.push(0u8);
        st.cstr_bufs.push(bytes);
        st.cstr_bufs.last().unwrap().as_ptr()
    })
}

extern "C" fn ar_write_handle(target_h: i64, new_val_h: i64) {
    if target_h < 3 { return; }
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let new_val = st.clone_value(new_val_h);
        if let Some(slot) = st.arena.get_mut(target_h as usize) {
            *slot = new_val;
        }
    });
}

// ── Feature-extension callbacks ──────────────────────────────────────────────

extern "C" fn ar_list_append(list_h: i64, item_h: i64) -> i64 {
    // Clone the Rc (cheap) while holding the borrow, then mutate outside.
    let (has_err, item, maybe_list) = STATE.with(|s| {
        let st = s.borrow();
        let item = st.clone_value(item_h);
        let maybe_list = if let Some(Value::List(list)) = st.arena.get(list_h as usize) {
            Some(list.clone())
        } else {
            None
        };
        (st.error.is_some(), item, maybe_list)
    });
    if has_err { return list_h; }
    if let Some(list_rc) = maybe_list {
        list_rc.borrow_mut().push(item);
    }
    list_h
}

extern "C" fn ar_raise_exc(type_h: i64, msg_h: i64) -> i64 {
    let (type_name, msg) = STATE.with(|s| {
        let st = s.borrow();
        let tn = match st.clone_value(type_h) {
            Value::Str(s) => s.to_string(),
            Value::Class(c) => c.name.clone(),
            Value::Instance(inst) => inst.borrow().class.name.clone(),
            _ => "Exception".to_string(),
        };
        let m = match st.clone_value(msg_h) {
            Value::Str(s) => s.to_string(),
            Value::None => String::new(),
            Value::Instance(inst) => {
                let b = inst.borrow();
                b.class.field_index.get("message").and_then(|&idx| {
                    match b.field_value(idx) {
                        Some(Value::Str(s)) => Some(s.to_string()),
                        _ => None,
                    }
                }).unwrap_or_default()
            }
            other => format!("{other:?}"),
        };
        (tn, m)
    });
    STATE.with(|s| s.borrow_mut().pending_raise = Some((type_name, msg)));
    TL_EXCEPTION
}

extern "C" fn ar_make_cell(init_h: i64) -> i64 {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(init_h);
        let cell = Rc::new(RefCell::new(vec![val]));
        st.push_value(Value::List(cell))
    })
}

extern "C" fn ar_get_cell(cell_h: i64) -> i64 {
    // Clone the Rc, release borrow, then access cell outside STATE lock.
    let maybe_list = STATE.with(|s| {
        if let Some(Value::List(list)) = s.borrow().arena.get(cell_h as usize) {
            Some(list.clone())
        } else {
            None
        }
    });
    if let Some(list_rc) = maybe_list {
        if let Some(v) = list_rc.borrow().first().cloned() {
            return push_handle(v);
        }
    }
    TL_NONE
}

extern "C" fn ar_set_cell(cell_h: i64, val_h: i64) {
    let (val, maybe_list) = STATE.with(|s| {
        let st = s.borrow();
        let val = st.clone_value(val_h);
        let maybe_list = if let Some(Value::List(list)) = st.arena.get(cell_h as usize) {
            Some(list.clone())
        } else {
            None
        };
        (val, maybe_list)
    });
    if let Some(list_rc) = maybe_list {
        if let Some(slot) = list_rc.borrow_mut().first_mut() {
            *slot = val;
        }
    }
}

extern "C" fn ar_call_method(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    }
    .to_owned();

    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // Native dispatch: check NATIVE_METHODS table first.
    if let Value::Instance(ref inst_rc) = obj {
        let class_name = inst_rc.borrow().class.name.clone();
        let fn_ptr = STATE.with(|s| {
            s.borrow().native_methods.get(&(class_name.clone(), name.clone())).copied()
        });
        if let Some(ptr) = fn_ptr {
            let mut all_args = Vec::with_capacity(1 + n_args as usize);
            all_args.push(obj_h);
            for i in 0..n_args as usize {
                all_args.push(unsafe { *args_ptr.add(i) });
            }
            unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(ptr);
                return func(all_args.as_ptr(), all_args.len() as i32);
            }
        }
    }

    // Interpreter fallback.
    let args: Vec<Value> = (0..n_args as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    let evaled: Vec<(Option<String>, Value, bool)> =
        args.into_iter().map(|v| (None, v, true)).collect();
    match interp.eval_method_call_evaled(obj, &name, evaled) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

// ── Typed field-read callbacks ─────────────────────────────────────────────

extern "C" fn ar_get_float_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> f64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0.0; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_float_field '{name}')")));
        return 0.0;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, name, None) {
        Ok(Value::Float(f)) => f,
        Ok(Value::Int(n)) => n as f64,
        Ok(Value::Bool(b)) => b as u8 as f64,
        Ok(_) | Err(_) => 0.0,
    }
}

extern "C" fn ar_get_int_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_int_field '{name}')")));
        return 0;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, name, None) {
        Ok(Value::Int(n)) => n,
        Ok(Value::Float(f)) => f as i64,
        Ok(Value::Bool(b)) => b as i64,
        Ok(_) | Err(_) => 0,
    }
}

extern "C" fn ar_flat_data_ptr(h: i64) -> i64 {
    if STATE.with(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().data.as_ptr() as i64,
        _ => 0,
    }
}

extern "C" fn ar_flat_len(h: i64) -> i64 {
    if STATE.with(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().len as i64,
        _ => 0,
    }
}

extern "C" fn ar_fn_trampoline(_fn_h: i64) -> *const () {
    ar_call_fn as *const ()
}

// ── Static callbacks instance ─────────────────────────────────────────────────

pub(crate) static CALLBACKS: ArCallbacks = ArCallbacks {
    make_int: ar_make_int,
    make_float: ar_make_float,
    make_bool: ar_make_bool,
    make_str: ar_make_str,
    make_list: ar_make_list,
    make_tuple: ar_make_tuple,
    make_dict: ar_make_dict,
    make_none: ar_make_none,
    is_truthy: ar_is_truthy,
    binop: ar_binop,
    unop: ar_unop,
    call_fn: ar_call_fn,
    get_attr: ar_get_attr,
    set_attr: ar_set_attr,
    subscript: ar_subscript,
    get_global: ar_get_global,
    iter_from: ar_iter_from,
    iter_next: ar_iter_next,
    is_type: ar_is_type,
    arena_save: ar_arena_save,
    arena_compact: ar_arena_compact,
    compact_many: ar_compact_many,
    to_int: ar_to_int,
    to_float: ar_to_float,
    deep_copy: ar_deep_copy,
    to_cstr: ar_to_cstr,
    write_handle: ar_write_handle,
    list_append: ar_list_append,
    raise_exc: ar_raise_exc,
    make_cell: ar_make_cell,
    get_cell: ar_get_cell,
    set_cell: ar_set_cell,
    call_method: ar_call_method,
    get_float_field: ar_get_float_field,
    get_int_field: ar_get_int_field,
    flat_data_ptr: ar_flat_data_ptr,
    flat_len: ar_flat_len,
    fn_trampoline: ar_fn_trampoline,
};

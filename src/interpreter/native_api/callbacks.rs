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

/// ネイティブ側から渡された `(ptr, len)` を `&str` にする。
///
/// 不正な UTF-8・負の長さ・長さのある NULL は、黙って受け取らずに呼び出しのエラー（`error`）にして
/// `None` を返す（ffi_value_exchange_plan.md 1-4・規約「掬いきれなかった選択肢はエラーにする」）。
/// ⚠ 以前は UTF-8 を検証せずに `str` にしていて、C の `char*`（CP932 などもありうる）が来ると未定義動作だった。
fn native_str<'a>(ptr: *const u8, len: i32, what: &str) -> Option<&'a str> {
    let bytes: &'a [u8] = if len == 0 {
        &[]
    } else if len < 0 || ptr.is_null() {
        set_native_error(format!("NativeError: invalid {what} from native code (pointer {ptr:?}, length {len})"));
        return None;
    } else {
        // SAFETY: ネイティブ側が `len` バイトの有効な領域を渡す契約（C ABI の呼び出し規約）。
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    };
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            set_native_error(format!("NativeError: {what} from native code is not valid UTF-8 ({e})"));
            None
        }
    }
}

fn set_native_error(msg: String) {
    STATE.with(|s| s.borrow_mut().error = Some(msg));
}

extern "C" fn ar_make_str(ptr: *const u8, len: i32) -> i64 {
    let Some(text) = native_str(ptr, len, "string") else { return TL_NONE };
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
            // ⚠ ここは `extern "C"` で `Result` を返せないので、**エラースロットへ積む**
            // （`take_error` が呼び出し境界で拾う）。黙って捨てない（B1-a）。
            if let Err(e) = crate::interpreter::Interpreter::dict_set_pure(&mut dict, k, v) {
                if st.error.is_none() {
                    st.error = Some(e);
                }
                break;
            }
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

/// 純スカラー引数のハンドル 1 個を u64 スロット値へデコードする（#83 で切り出し）。
///
/// ⚠ **アリーナから直接読む**（`Value` のクローンを挟まない）。これが `TypedArgs::marshal` を
/// 通さない唯一の理由で、STATE アクセスが ~8 回 → 1〜2 回になる（#76 で残した例外）。
/// 型の許容表は共有マーシャラと同一に保つこと（`I64` は int のみ・`F64` は float と int）。
#[inline]
fn scalar_handle_slot(st: &NativeCallState, h: i64, ty: &crate::interpreter::value::AbiTy) -> Option<u64> {
    use crate::interpreter::value::AbiTy;
    let (iv, fv): (Option<i64>, Option<f64>) = match h {
        x if (3..INT_CACHE_BASE as i64).contains(&x) => (Some(x - 3), None),
        x if x >= INT_CACHE_BASE as i64 => match st.arena.get(x as usize) {
            Some(Value::Int(v)) => (Some(*v), None),
            Some(Value::Float(f)) => (None, Some(*f)),
            _ => return None,
        },
        _ => return None, // None/Bool/sentinel → ハンドル経路へ
    };
    match (iv, fv, ty) {
        (Some(v), _, AbiTy::I64) => Some(v as u64),
        (Some(v), _, AbiTy::F64) => Some((v as f64).to_bits()),
        (_, Some(f), AbiTy::F64) => Some(f.to_bits()),
        _ => None,
    }
}

/// 全引数が純スカラーのときのマーシャリング（#83 で切り出し）。`false` ＝ ハンドル経路へ。
///
/// ⚠ **1 回の STATE ボローで全引数を処理する**のが要点（借用を跨がないこと）。
#[inline]
fn marshal_scalar_handles(
    ta: &mut crate::interpreter::value::TypedArgs,
    params: &[crate::interpreter::value::AbiTy],
    args_ptr: *const i64,
    n: usize,
) -> bool {
    STATE.with(|s| {
        let st = s.borrow();
        for (i, ty) in params.iter().enumerate().take(n) {
            let h = unsafe { *args_ptr.add(i) };
            match scalar_handle_slot(&st, h, ty) {
                Some(v) => ta.slots[i] = v,
                None => return false,
            }
        }
        true
    })
}

/// 統一 typed ABI 最速パス（ネイティブ → C DLL）の試行（#83 で `ar_call_fn` から切り出し）。
///
/// 引数ハンドルを 1 回の STATE ボローで u64 スロットへ展開し、`{name}_typed` を直接呼ぶ。
/// `enter_native_call` / `exit_native_call`・per-arg unmarshal コールバック・結果 marshal が
/// すべて消える（STATE アクセス ~8 回 → 1〜2 回）。
///
/// - `Some(h)`: この経路で決着した（エラーも `STATE.error` に載せて `TL_NONE` を返す形で含む）
/// - `None`: この経路に載らない ⇒ **呼び出し側がハンドル経路へ落とす**
///
/// ⚠ この層は named-variable の概念がない（アリーナハンドル止まり）ため、
/// `named_mut` は常に空＝**書き戻しなし**（安全側・#76）。
///
/// ⚠⚠ **#83 以前はこれが `ar_call_fn` に直書きで、ガードが 4 段入れ子になっていた**
/// （制御ネスト 8 ＝ src 最深）。早期 return へ反転してある — **`if` を足して深くしないこと**。
///
/// ⚠⚠ **`#[inline(always)]` を外さないこと**（#83）。呼び出し元は `ar_call_fn` の 1 箇所だけで、
/// #83 以前は文字どおりインライン展開されていたコード。切り出しただけで生成コードが変わると
/// **cdll ベンチが 0.94x に動く**（実測。⚠ cdll は `ar_call_fn` を通らないので
/// **これは経路の速度ではなくコード配置の話** — 属性を戻すだけで 1.04x に振れた）。
#[inline(always)]
fn try_typed_fast_call(fn_val: &Value, args_ptr: *const i64, n_args: i32) -> Option<i64> {
    use crate::interpreter::value::{AbiTy, Marshalled, TypedArgs};
    use std::sync::atomic::Ordering;

    let Value::NativeFunction(fn_ref) = fn_val else {
        return None;
    };
    let sig = fn_ref.typed_sig.as_ref()?;
    let typed_ptr = fn_ref.typed_fn_ptr.load(Ordering::Relaxed);
    let n = n_args as usize;
    if typed_ptr == 0 || n != sig.params.len() || n > 16 {
        return None;
    }

    // 数値（`I64` / `F64`）以外の引数（構造体ポインタ・書き込みポインタ・`bool`・`bytes`・`Ptr`・`ArPtrProtocol`）を
    // 含む場合は、値に戻して共有の `marshal` を通す（P3/P4・4-4 — .claude/skills/c-abi-interop/SKILL.md）。
    let has_ptr = sig.params.iter().any(|p| !matches!(p, AbiTy::I64 | AbiTy::F64));
    // ⚠⚠ **move 禁止**（`slots` が `out_locals` を指す）。ここに置いたまま使う。
    let mut ta = TypedArgs::new();

    let ok = if !has_ptr {
        marshal_scalar_handles(&mut ta, &sig.params, args_ptr, n)
    } else {
        // 1 回の STATE ボローでハンドルを Value に解決（Instance は Rc クローンのみで安価）、
        // 以降は STATE の外で処理する。マーシャリング本体は共有実装（#76）。
        let vals: Vec<Value> = STATE.with(|s| {
            let st = s.borrow();
            (0..n).map(|i| st.clone_value(unsafe { *args_ptr.add(i) })).collect()
        });
        match ta.marshal(&vals, &sig.params, &[]) {
            Ok(Marshalled::Ready) => true,
            Ok(Marshalled::TypeMismatch) => false,
            Err(e) => {
                STATE.with(|s| s.borrow_mut().error = Some(e));
                return Some(TL_NONE);
            }
        }
    };
    if !ok {
        return None; // 型不一致 → 既存のハンドル経路にフォールバック
    }

    // 呼び出し本体は 4 経路で共有（#54 / #76）。⚠ この層はエラーを戻り値で返せないので
    // `STATE.error` に載せて TL_NONE を返す — 違いはそこだけ。
    // SAFETY: `typed_ptr` は typed_sig 付き（`build_cpp_typed_sig` 検証済み）の関数。
    // 戻り値は呼び出し本体が引数のバッファの生きているうちにデコードする（4-4・`invoke_typed_abi`）。
    // ⚠ OutPtr の書き戻しは無い（名前が無いので安全側 ＝ `ta.out_wb` は常に空）。
    match unsafe {
        crate::interpreter::value::invoke_typed_abi(typed_ptr, &ta.slots, std::mem::take(&mut ta.cleanups), &sig.ret)
    } {
        // 小整数はキャッシュ済みハンドル（`push_handle` が STATE を触らない）・`None` は TL_NONE
        Ok(v) => Some(push_handle(v)),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            Some(TL_NONE)
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
    // 本体は下の `try_typed_fast_call`（#83 で切り出した）。`None` ＝ この経路に載らない
    // ので、そのままハンドル経路へ落ちる。
    if let Some(h) = try_typed_fast_call(&fn_val, args_ptr, n_args) {
        return h;
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
                                STATE.with(|s| s.borrow_mut().error = Some(fn_ref.missing_symbol_error(&e)));
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
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return TL_NONE };
    let name = name.to_owned();
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
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return };
    let name = name.to_owned();
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
    let Some(name) = native_str(name_ptr, name_len, "global name") else { return TL_NONE };
    let name = name.to_owned();
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
    let Some(name) = native_str(name_ptr, name_len, "type name") else { return TL_NONE };
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

/// ネイティブの `int` を取り出す（C / Rust の整数の引数・ネイティブコンパイルの生成コードの開封）。
///
/// 数値どうしは C の暗黙の変換と同じ（`float` は切り捨て・`bool` は 0 / 1・幅への切り詰めは中継コード）。
/// ⚠ **数値でない値（`None`・`str`・`list`・`Ptr` …）はエラー**を設定して 0 を返す（7-12）。以前は黙って 0 にしていて、
///   素の `function` 型の変数を通して `fp.fp_is_even("abc")` が `True` を返していた（実測）。中継コードは
///   `pending_error` を見て C / Rust の関数を呼ばずに戻る。
extern "C" fn ar_to_int(h: i64) -> i64 {
    match h {
        TL_TRUE => 1,
        TL_FALSE => 0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => n - 3,
        n => {
            let r = STATE.with(|s| match s.borrow().arena.get(n as usize) {
                Some(Value::Int(v)) => Some(*v),
                Some(Value::UInt(v)) => Some(*v as i64),
                Some(Value::Float(f)) => Some(*f as i64),
                Some(Value::Bool(b)) => Some(*b as i64),
                _ => None,
            });
            r.unwrap_or_else(|| {
                let got = handle_type_name(n);
                set_argument_error(format!("TypeError: a native int argument must be a number, not '{got}'"));
                0
            })
        }
    }
}

/// ネイティブの `float` を取り出す。規則は [`ar_to_int`] と同じ（`int` は広げる・数値でない値はエラー・7-12）。
extern "C" fn ar_to_float(h: i64) -> f64 {
    match h {
        TL_TRUE => 1.0,
        TL_FALSE => 0.0,
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => (n - 3) as f64,
        n => {
            let r = STATE.with(|s| match s.borrow().arena.get(n as usize) {
                Some(Value::Int(v)) => Some(*v as f64),
                Some(Value::UInt(v)) => Some(*v as f64),
                Some(Value::Float(f)) => Some(*f),
                Some(Value::Bool(b)) => Some(*b as u8 as f64),
                _ => None,
            });
            r.unwrap_or_else(|| {
                let got = handle_type_name(n);
                set_argument_error(format!("TypeError: a native float argument must be a number, not '{got}'"));
                0.0
            })
        }
    }
}

/// ネイティブの `bool` を取り出す（7-12）。`True` / `False` だけ。以前は中継コードが `h == TL_TRUE` と比べていて、
/// `True` 以外は何でも黙って偽になっていた。
extern "C" fn ar_to_bool(h: i64) -> i32 {
    match h {
        TL_TRUE => 1,
        TL_FALSE => 0,
        n => {
            let got = handle_type_name(n);
            set_argument_error(format!("TypeError: a native bool argument must be a bool, not '{got}'"));
            0
        }
    }
}

/// 引数の変換でエラーが設定されたか（7-12・`ArCallbacks::pending_error` の doc）。
extern "C" fn ar_pending_error() -> i32 {
    STATE.with(|s| s.borrow().error.is_some() as i32)
}

/// `char*` / `wchar_t*` の戻り値（4-3・D-9）。`len` バイトを `bytes` へコピーする（借用なので解放しない・D-7）。
extern "C" fn ar_make_bytes(ptr: *const u8, len: i64) -> i64 {
    if len < 0 || (ptr.is_null() && len > 0) {
        set_argument_error(format!("RuntimeError: invalid byte string from native code (len {len})"));
        return TL_NONE;
    }
    // SAFETY: 中継コードが NUL 終端の長さ（`wchar_t` は 2 倍）を数えて渡す。長さ 0 は NULL でも空。
    let data = if len == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(ptr, len as usize) }.to_vec() };
    push_handle(Value::bytes(data))
}

/// ポインタの戻り値（4-3・D-7・D-8）。借用の `Ptr[elem]`（Arrow は解放しない）。NULL は `None`。
/// ⚠ DLL への参照は持たせない: C の DLL は中継 DLL が `LoadLibrary` したまま解放しないので、プロセスが終わるまで
///   アンロードされない（`PtrProtocol` の解放関数を持つ所有の `Ptr` は 4-5 で扱う）。
extern "C" fn ar_make_ptr(address: u64, elem: *const u8, elem_len: i32) -> i64 {
    if address == 0 {
        return TL_NONE;
    }
    let Some(elem) = native_str(elem, elem_len, "pointer element type") else { return TL_NONE };
    match crate::interpreter::PtrHandle::new(address, elem, 0, None, crate::interpreter::PtrOwner::Borrowed, None) {
        Ok(h) => push_handle(Value::Ptr(std::sync::Arc::new(h))),
        Err(e) => {
            set_argument_error(e);
            TL_NONE
        }
    }
}

/// `const char*` の引数（4-3・D-13）。`bytes` は NUL を付けたコピー（最外のネイティブ呼び出しが終わるまで有効）、
/// `None` は NULL（D-8）。`str` は文字コードを書かせるためにエラー（黙って UTF-8 にしない）。NUL を含む `bytes` は
/// C の側で途中で切れるのでエラー。誤りは `pending_error` で拾われ、C の関数は呼ばれない（7-12）。
extern "C" fn ar_bytes_arg(h: i64) -> *const u8 {
    // 規則は型つき経路と共有（`value::c_string_arg`）
    match crate::interpreter::value::c_string_arg(&clone_value_at(h)) {
        Ok(None) => std::ptr::null(),
        Ok(Some(buf)) => STATE.with(|s_tls| {
            let mut st = s_tls.borrow_mut();
            st.cstr_bufs.push(buf);
            st.cstr_bufs.last().unwrap().as_ptr()
        }),
        Err(e) => {
            set_argument_error(e);
            std::ptr::null()
        }
    }
}

/// `void*` / 構造体ポインタの引数（4-3・D-3・D-8）。`Ptr` のアドレスか、`None` の 0。要素型は作られたときの型で照合する
/// （`void` はどの `Ptr` も受ける）。解放済み・C の側で解放済みの `Ptr` はエラー。
extern "C" fn ar_ptr_arg(h: i64, elem: *const u8, elem_len: i32) -> u64 {
    let Some(elem) = native_str(elem, elem_len, "pointer element type") else { return 0 };
    // 規則は型つき経路と共有（`value::ptr_arg_address`）
    crate::interpreter::value::ptr_arg_address(&clone_value_at(h), elem).unwrap_or_else(|e| {
        set_argument_error(e);
        0
    })
}

/// `ArPtrProtocol` の引数（ハンドル経路・4-4）。`Ptr` か `None` から構造体を組み立てて `out` へ書く（規則は型つき経路と共有の
/// `value::protocol_arg`）。要素の型名の文字列は最外のネイティブ呼び出しが終わるまで生かす。
extern "C" fn ar_protocol_arg(h: i64, out: *mut u8, writable: i32) {
    match crate::interpreter::value::protocol_arg(&clone_value_at(h), writable != 0) {
        Ok(arg) => {
            // SAFETY: `out` は中継コードの `_ArPtrProtocol`（`PtrProtocolRaw` と同じ並び・単体テストが守る）を指す
            unsafe { *(out as *mut crate::interpreter::value::PtrProtocolRaw) = *arg.raw };
            STATE.with(|s| s.borrow_mut().protocol_args.push((h, arg)));
        }
        Err(e) => set_argument_error(e),
    }
}

/// `ArPtrProtocol*` の引数の呼び出し後（ハンドル経路・4-4）。C が `address` を 0 に書き戻していたら、その `Ptr` を使えなくする
/// （解放関数はもう呼ばない・D-3）。
extern "C" fn ar_protocol_done(h: i64, address_after: u64) {
    // `protocol_arg` が組み立てた引数を探して、型つき経路と同じ後処理（`PtrProtocolArg::finish`）をする:
    // `Ptr` を無効にする・`list` へ書き戻す（4-7）
    let arg = STATE.with(|s| {
        let mut st = s.borrow_mut();
        let i = st.protocol_args.iter().rposition(|(ah, _)| *ah == h)?;
        Some(st.protocol_args.remove(i).1)
    });
    if let Some(mut arg) = arg {
        arg.set_address_after(address_after);
        arg.finish();
    }
}

/// `char*` の出力バッファ（4-7・D-14）: `bytes` のコピーを作る（NUL を含んでよい）。`None` は NULL。
extern "C" fn ar_bytes_buf_arg(h: i64, len: *mut i64) -> *mut u8 {
    let data = match clone_value_at(h) {
        Value::None => {
            unsafe { *len = 0 };
            return std::ptr::null_mut();
        }
        Value::Bytes(b) => b,
        other => {
            set_argument_error(format!(
                "TypeError: a C output buffer (`char*`) parameter needs bytes (e.g. bytes(64)), not '{}'",
                crate::interpreter::ops::typecheck::runtime_type_name(&other)
            ));
            unsafe { *len = 0 };
            return std::ptr::null_mut();
        }
    };
    // SAFETY: 中継コードの局所変数
    unsafe { *len = data.len() as i64 };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let mut buf = data.to_vec();
        buf.push(0);
        st.cstr_bufs.push(buf);
        st.cstr_bufs.last_mut().unwrap().as_mut_ptr()
    })
}

/// `ArPtrProtocol` の戻り値（ハンドル経路・4-5）。規則は型つき経路と共有（`value::ptr_from_protocol`）。
extern "C" fn ar_protocol_ret(raw: *const u8, structs: *const u8, structs_len: i32) -> i64 {
    let names: Vec<std::sync::Arc<str>> = match native_str(structs, structs_len, "struct names") {
        Some(s) => s.split(',').filter(|n| !n.is_empty()).map(std::sync::Arc::from).collect(),
        None => return TL_NONE,
    };
    // SAFETY: `raw` は中継コードの `_ArPtrProtocol`（`PtrProtocolRaw` と同じ並び）を指す
    let raw = unsafe { &*(raw as *const crate::interpreter::value::PtrProtocolRaw) };
    match crate::interpreter::value::ptr_from_protocol(raw, &names) {
        Ok(v) => push_handle(v),
        Err(e) => {
            set_argument_error(e);
            TL_NONE
        }
    }
}

/// クラスのハンドルから C の構造体のクラスを取り出す（4-6）。
fn struct_class_at(cls_h: i64) -> Result<std::rc::Rc<crate::interpreter::ClassValue>, String> {
    match clone_value_at(cls_h) {
        Value::Class(c) => Ok(c),
        other => Err(format!(
            "RuntimeError: expected the class of a C struct, got '{}'",
            crate::interpreter::ops::typecheck::runtime_type_name(&other)
        )),
    }
}

/// 値返しの構造体（正確なレイアウト・4-6）: C のバイト列をそのまま写したインスタンス（コンストラクタは呼ばない）。
/// ⚠ 以前はフィールドを 1 つずつハンドルにしてコンストラクタへ渡して作り直していた。
extern "C" fn ar_make_raw_instance(cls_h: i64, ptr: *const u8, len: i64) -> i64 {
    let r = struct_class_at(cls_h).and_then(|class| {
        // SAFETY: 中継コードが値返しの構造体の局所変数と、その `size_of` を渡す
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len.max(0) as usize) };
        crate::interpreter::InstanceData::from_c_bytes(class, bytes)
    });
    match r {
        Ok(inst) => push_handle(Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(inst)))),
        Err(e) => {
            set_argument_error(e);
            TL_NONE
        }
    }
}

/// 値返しの構造体（ポインタのフィールドあり・4-6）: 変換済みのフィールドの値（宣言順）を入れたインスタンス。
/// コンストラクタは呼ばない。フィールドの変換で誤りが出ていたら（`pending_error`）何も作らない。
extern "C" fn ar_make_instance(cls_h: i64, handles: *const i64, n: i32) -> i64 {
    if ar_pending_error() != 0 {
        return TL_NONE;
    }
    let class = match struct_class_at(cls_h) {
        Ok(c) => c,
        Err(e) => {
            set_argument_error(e);
            return TL_NONE;
        }
    };
    if n < 0 || n as usize != class.field_count {
        set_argument_error(format!(
            "RuntimeError: struct '{}' has {} fields, but {n} values were given",
            class.name, class.field_count
        ));
        return TL_NONE;
    }
    let mut inst = crate::interpreter::InstanceData::new_empty(class, 0);
    for i in 0..n as usize {
        // SAFETY: 中継コードが `n` 個のハンドルの配列を渡す
        let v = clone_value_at(unsafe { *handles.add(i) });
        inst.store_field(i, v, true);
    }
    push_handle(Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(inst))))
}

/// ハンドルの値の型名（エラーの表示用）。⚠ `None` / `True` / `False` / 小さい整数はアリーナに入らず番号で表す
/// （`push_value`）ので、番号で見分けてからアリーナを引く。
fn handle_type_name(h: i64) -> String {
    use crate::interpreter::ops::typecheck::runtime_type_name;
    match h {
        TL_NONE => runtime_type_name(&Value::None).to_string(),
        TL_TRUE | TL_FALSE => "bool".to_string(),
        n if n >= 3 && (n as usize) < INT_CACHE_BASE => "int".to_string(),
        n => STATE.with(|s| {
            s.borrow().arena.get(n as usize).map_or("invalid handle".to_string(), |v| runtime_type_name(v).to_string())
        }),
    }
}

/// 引数の変換の誤りを設定する。⚠ **最初の誤りを残す**（2 つ目の引数の誤りで 1 つ目を上書きしない）。
fn set_argument_error(msg: String) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_none() {
            st.error = Some(msg);
        }
    });
}

// ── cpp-bridge helpers ───────────────────────────────────────────────────────

/// Arrow の値を C の文字列（NUL 終端・呼び出しの間だけ有効）にする。
///
/// ⚠ 渡せるのは `str` だけ。以前は `None` などを黙って空文字列にしていた（ffi_value_exchange_plan.md 1-2）。
///   `str` 以外と、NUL を含む `str`（C 側では途中で切れる）はエラーを設定して **NULL** を返す。
///   C/C++ の中継コードは NULL なら C 関数を呼ばずに戻り、呼び出しの後でエラーが例外になる。
///   Rust の crate の中継コード（`rs_loader`）は NULL を空文字列として関数を呼ぶが、その後でエラーになる。
extern "C" fn ar_to_cstr(h: i64) -> *const u8 {
    let s = match clone_value_at(h) {
        Value::Str(s) => s.to_string(),
        other => {
            set_native_error(format!(
                "TypeError: a C string (`const char*`) parameter needs a str, not '{}'",
                crate::interpreter::ops::typecheck::runtime_type_name(&other)
            ));
            return std::ptr::null();
        }
    };
    if s.contains('\0') {
        set_native_error(
            "ValueError: a str passed to a C string (`const char*`) parameter must not contain NUL (the C side would cut it there)"
                .to_string(),
        );
        return std::ptr::null();
    }
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
    let Some(name) = native_str(name_ptr, name_len, "method name") else { return TL_NONE };
    let name = name.to_owned();

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
    let Some(name) = native_str(name_ptr, name_len, "field name") else { return 0.0 };
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
    let Some(name) = native_str(name_ptr, name_len, "field name") else { return 0 };
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
    to_bool: ar_to_bool,
    pending_error: ar_pending_error,
    make_bytes: ar_make_bytes,
    make_ptr: ar_make_ptr,
    bytes_arg: ar_bytes_arg,
    ptr_arg: ar_ptr_arg,
    protocol_arg: ar_protocol_arg,
    protocol_done: ar_protocol_done,
    protocol_ret: ar_protocol_ret,
    make_raw_instance: ar_make_raw_instance,
    make_instance: ar_make_instance,
    bytes_buf_arg: ar_bytes_buf_arg,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// 正しい UTF-8 は `str` になる（日本語を含む）。
    #[test]
    fn make_str_accepts_valid_utf8() {
        let text = "日本語 abc";
        let h = ar_make_str(text.as_ptr(), text.len() as i32);
        assert!(take_error().is_none());
        assert!(matches!(clone_value_at(h), Value::Str(s) if &*s == text));
        // 長さ 0 は NULL でも空文字列（`from_raw_parts` に NULL を渡さない）
        let h = ar_make_str(std::ptr::null(), 0);
        assert!(take_error().is_none());
        assert!(matches!(clone_value_at(h), Value::Str(s) if s.is_empty()));
    }

    /// 不正な UTF-8・負の長さ・長さのある NULL は、黙って受け取らずにエラーになる（1-4）。
    #[test]
    fn make_str_rejects_invalid_input() {
        let cp932 = [0x93u8, 0xfa, 0x96, 0x7b]; // 「日本」の CP932
        assert_eq!(ar_make_str(cp932.as_ptr(), cp932.len() as i32), TL_NONE);
        assert!(take_error().is_some_and(|e| e.contains("not valid UTF-8")));

        assert_eq!(ar_make_str(b"abc".as_ptr(), -1), TL_NONE);
        assert!(take_error().is_some_and(|e| e.contains("invalid string")));

        assert_eq!(ar_make_str(std::ptr::null(), 3), TL_NONE);
        assert!(take_error().is_some_and(|e| e.contains("invalid string")));
    }

    /// C の文字列へは str だけ。None などや NUL を含む str は、黙って空文字列にせずエラーで NULL（1-2）。
    #[test]
    fn to_cstr_accepts_only_str_without_nul() {
        let h = ar_make_str(b"abc".as_ptr(), 3);
        let p = ar_to_cstr(h);
        assert!(!p.is_null());
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_bytes(), b"abc");
        assert!(take_error().is_none());

        assert!(ar_to_cstr(TL_NONE).is_null());
        assert!(take_error().is_some_and(|e| e.contains("needs a str") && e.contains("NoneType")));

        let h = ar_make_str(b"a\0b".as_ptr(), 3);
        assert!(ar_to_cstr(h).is_null());
        assert!(take_error().is_some_and(|e| e.contains("NUL")));
    }

    /// 数値でない値は 0 にせずエラーにする。最初の誤りを残す。数値どうしは C の暗黙の変換のまま（7-12）。
    #[test]
    fn to_int_and_to_float_reject_non_numbers() {
        let s = ar_make_str(b"abc".as_ptr(), 3);
        assert_eq!(ar_pending_error(), 0);
        assert_eq!(ar_to_int(s), 0);
        assert_eq!(ar_pending_error(), 1);
        // 2 つ目の誤りで 1 つ目を上書きしない
        assert_eq!(ar_to_float(TL_NONE), 0.0);
        assert!(take_error().is_some_and(|e| e.contains("int argument must be a number") && e.contains("'str'")));
        assert_eq!(ar_pending_error(), 0);
        assert_eq!(ar_to_float(TL_NONE), 0.0);
        assert!(take_error().is_some_and(|e| e.contains("float argument") && e.contains("NoneType")));
        let xs = ar_make_list(std::ptr::null(), 0);
        assert_eq!(ar_to_int(xs), 0);
        assert!(take_error().is_some_and(|e| e.contains("'list'")));
        // 数値どうし
        assert_eq!(ar_to_int(push_handle(Value::Float(3.7))), 3);
        assert_eq!(ar_to_int(TL_TRUE), 1);
        assert_eq!(ar_to_int(push_handle(Value::Int(1 << 40))), 1 << 40);
        assert_eq!(ar_to_float(push_handle(Value::Int(5))), 5.0);
        assert!(take_error().is_none());
    }

    /// `bool` の引数は `True` / `False` だけ。以前は `True` 以外を黙って偽にしていた（7-12）。
    #[test]
    fn to_bool_accepts_only_bools() {
        assert_eq!(ar_to_bool(TL_TRUE), 1);
        assert_eq!(ar_to_bool(TL_FALSE), 0);
        assert!(take_error().is_none());
        // 小さい整数はアリーナに入らない（番号で表す）ので、型名を取り違えない
        assert_eq!(ar_to_bool(push_handle(Value::Int(1))), 0);
        assert!(take_error().is_some_and(|e| e.contains("must be a bool") && e.contains("'int'")));
        assert_eq!(ar_to_bool(TL_NONE), 0);
        assert!(take_error().is_some_and(|e| e.contains("NoneType")));
    }

    /// `char*` の戻り値は bytes、ほかのポインタは借用の Ptr、NULL は None（4-3）。
    #[test]
    fn raw_pointer_returns() {
        let text = b"hi\0";
        let h = ar_make_bytes(text.as_ptr(), 2);
        assert!(matches!(clone_value_at(h), Value::Bytes(b) if &*b == b"hi"));
        assert!(matches!(clone_value_at(ar_make_bytes(std::ptr::null(), 0)), Value::Bytes(b) if b.is_empty()));
        assert_eq!(ar_make_ptr(0, b"void".as_ptr(), 4), TL_NONE);
        let p = ar_make_ptr(0x1000, b"Db".as_ptr(), 2);
        match clone_value_at(p) {
            Value::Ptr(p) => {
                assert_eq!(p.elem_type(), "Db");
                assert!(p.release().unwrap_err().contains("borrowed"), "Arrow never frees a raw pointer");
            }
            other => panic!("{other:?}"),
        }
        assert!(take_error().is_none());
    }

    /// `const char*` の引数は bytes か None。str・NUL を含む bytes はエラー（4-3・D-13）。
    #[test]
    fn bytes_arg_takes_bytes_or_none() {
        let b = push_handle(Value::bytes(b"abc".to_vec()));
        let p = ar_bytes_arg(b);
        assert_eq!(unsafe { std::ffi::CStr::from_ptr(p as *const std::ffi::c_char) }.to_bytes(), b"abc");
        assert!(ar_bytes_arg(TL_NONE).is_null());
        assert!(take_error().is_none(), "None is NULL, not an error");
        let s = ar_make_str(b"abc".as_ptr(), 3);
        assert!(ar_bytes_arg(s).is_null());
        assert!(take_error().is_some_and(|e| e.contains("needs bytes, not 'str'") && e.contains("encode")));
        let nul = push_handle(Value::bytes(b"a\0b".to_vec()));
        assert!(ar_bytes_arg(nul).is_null());
        assert!(take_error().is_some_and(|e| e.contains("NUL")));
    }

    /// `void*` / 構造体ポインタの引数は Ptr（要素型を照合）か None（4-3）。
    #[test]
    fn ptr_arg_checks_the_element_type() {
        let db = ar_make_ptr(0x2000, b"Db".as_ptr(), 2);
        assert_eq!(ar_ptr_arg(db, b"Db".as_ptr(), 2), 0x2000);
        assert_eq!(ar_ptr_arg(db, b"void".as_ptr(), 4), 0x2000, "any Ptr goes to void*");
        assert_eq!(ar_ptr_arg(TL_NONE, b"Db".as_ptr(), 2), 0);
        assert!(take_error().is_none());
        assert_eq!(ar_ptr_arg(db, b"File".as_ptr(), 4), 0);
        assert!(take_error().is_some_and(|e| e.contains("Ptr[File]") && e.contains("Ptr[Db]")));
        assert_eq!(ar_ptr_arg(push_handle(Value::Int(1 << 40)), b"void".as_ptr(), 4), 0);
        assert!(take_error().is_some_and(|e| e.contains("needs a Ptr or None") && e.contains("'int'")));
    }

    /// 名前を受け取るコールバックも同じ検査を通る（不正なら呼び出しのエラー）。
    #[test]
    fn name_callbacks_reject_invalid_utf8() {
        let bad = [0xffu8, 0xfe];
        assert_eq!(ar_get_global(bad.as_ptr(), bad.len() as i32), TL_NONE);
        assert!(take_error().is_some_and(|e| e.contains("global name") && e.contains("not valid UTF-8")));
    }
}

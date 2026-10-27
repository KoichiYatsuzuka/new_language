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
    // VM の条件と同じ `eval_truthy`（`__bool__` を呼ぶ・native_object_access_plan.md 1-14）。⚠ 以前は `is_truthy` で
    // `__bool__` を見ていなかった。失敗は呼び出しの出口で例外になる（ほかのコールバックと同じ）。
    let interp = unsafe { &mut *interp_ptr };
    match interp.eval_truthy(&v) {
        Ok(b) => i32::from(b),
        Err(e) => {
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                if st.error.is_none() {
                    st.error = Some(e);
                }
            });
            0
        }
    }
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
    // VM の `BinOp` と同じ `apply_binop_dyn`（`__add__` / `__eq__` などを呼ぶ・`in` の `__eq__`・1-14）。
    // ⚠ 以前は `apply_binop` で演算子のメソッドを見ず、`a + b` が TypeError、`a == b` が `__eq__` でなく構造の比較になっていた。
    let interp = unsafe { &mut *interp_ptr };
    match interp.apply_binop_dyn(&ast_op, lhs, rhs) {
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
    // VM の単項演算と同じ `apply_unary_dyn`（`__neg__` などを呼ぶ・1-14）。⚠ 以前は `apply_unary` で見ていなかった。
    let interp = unsafe { &mut *interp_ptr };
    match interp.apply_unary_dyn(&ast_op, operand) {
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
    // ⚠ `.arc` の関数で個数が合わない（既定値を省いた）呼び出しは下の遅い道で宣言どおりに束縛する（1-11）。
    //   生成した `_tl` は渡された個数を見ずに仮引数の数だけ読むので、ここで素通しすると引数の配列の外を読む。
    if let Value::NativeFunction(ref fn_ref) = fn_val {
        if !interp_ptr.is_null() && (fn_ref.globals.is_none() || n_args as usize == fn_ref.n_params) {
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
            // `let` 仮引数は `copy_value` で複製してから渡す（6-2・D-8）。ここには引数の可変性が無いので保守的に
            // `mut` とみなす。⚠ 以前はハンドルを素通しし、`let` で受けた値が呼び出し元の後の変更で変わりえた（1-5 と同じ形）。
            let mut owned: Vec<i64> = (0..n_args as usize).map(|i| unsafe { *args_ptr.add(i) }).collect();
            // 複製を作る前のアリーナの長さ（1-15）。呼び出しの後でここまで巻き戻し、複製を呼び出しごとに捨てる。
            let mut copied_from: Option<u64> = None;
            for (i, h) in owned.iter_mut().enumerate() {
                if !fn_ref.param_mutabilities.get(i).copied().unwrap_or(true)
                    && fn_ref.ptr_params.get(i).copied().unwrap_or(crate::interpreter::PtrParam::None)
                        == crate::interpreter::PtrParam::None
                {
                    copied_from.get_or_insert_with(|| STATE.with(|s| s.borrow().arena.len() as u64));
                    match unsafe { &mut *interp_ptr }.copy_value(clone_value_at(*h)) {
                        Ok(v) => *h = push_handle(v),
                        Err(e) => {
                            STATE.with(|s| s.borrow_mut().error = Some(e));
                            abort_native_call(is_outermost);
                            return TL_NONE;
                        }
                    }
                }
            }
            // コンパイル済みモジュールの関数はそのモジュールの大域で走る（1-7）
            let prev = unsafe { &mut *interp_ptr }.enter_native_globals(fn_ref.globals);
            let result_h = unsafe {
                let f: unsafe extern "C" fn(*const i64, i32) -> i64 =
                    std::mem::transmute(fn_ptr);
                f(owned.as_ptr(), n_args)
            };
            unsafe { &mut *interp_ptr }.leave_native_globals(prev);

            if STATE.with(|s| s.borrow().error.is_some()) {
                abort_native_call(is_outermost);
                return TL_NONE;
            }
            // 複製を呼び出しごとに捨てる（1-15）。いちばん外の呼び出しなら下の `exit_native_call` が全部片づける
            let result_h = match copied_from {
                Some(saved) if !is_outermost => ar_arena_compact(result_h, saved),
                _ => result_h,
            };
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

/// field 13。⚠ **`import[rs]` の中継コードが Arrow 側の写しを更新する経路**（`set_attr_val`）。
/// 生成された `set_{field}` は Rust の値を書き換えた後でこれを呼ぶので、`attr_assign_evaled`
/// （rs の setter へ振り分ける）にすると setter へ再入する。コンパイル済み Arrow コードの代入は
/// `ar_assign_attr`（field 51・native_object_access_plan.md 1-4）。
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

/// コンパイル済み Arrow コードの `obj.attr = v`（field 51・native_object_access_plan.md 1-4）。
/// VM の `SetAttr` と同じ `attr_assign_evaled`。⚠ 以前は `set_attr_val` を通っていたので、
/// `static mut` へのインスタンス経由の代入（`self.total = ..`）が「has no field」で落ちていた。
extern "C" fn ar_assign_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return };
    let (has_err, obj, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.attr_assign_evaled(obj, name, val) {
        STATE.with(|s| s.borrow_mut().error = Some(e));
    }
}

/// `obj.attr = v`（IC 付き・field 61・native_object_access_plan.md 2-3）。VM の `SetAttr` と同じ `attr_assign_ic`。
/// `cell` は生成コードの箇所ごとの書き込みの IC（読み出しとは別のセル）。`AtomicU64`（Relaxed）として読み書きし、
/// 一時の `AttrCache` に写して渡し、埋まった値を書き戻す（`read_attr_ic` と同じ）。
extern "C" fn ar_assign_attr_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64, cell: *mut u64) {
    let val = clone_value_at(val_h);
    assign_attr_ic_value(obj_h, name_ptr, name_len, val, cell);
}

/// `obj.attr = <int>`（IC 付き・field 65・4-2）。値はハンドルにせず `Value::Int` で渡す（中身は field 61 と同じ）。
extern "C" fn ar_assign_int_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, v: i64, cell: *mut u64) {
    assign_attr_ic_value(obj_h, name_ptr, name_len, Value::Int(v), cell);
}

/// `obj.attr = <float>`（IC 付き・field 66・4-2）。
extern "C" fn ar_assign_float_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, v: f64, cell: *mut u64) {
    assign_attr_ic_value(obj_h, name_ptr, name_len, Value::Float(v), cell);
}

/// fields 61 / 65 / 66 の本体（値の受け取り方だけが違う）。
fn assign_attr_ic_value(obj_h: i64, name_ptr: *const u8, name_len: i32, val: Value, cell: *mut u64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(name) = site_name(name_ptr, name_len, cell, "attribute name") else { return };
    if cell.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: no inline cache cell for attribute '{name}'")));
        return;
    }
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let cache = crate::ast::AttrCache(std::cell::Cell::new(cell.load(Ordering::Relaxed)));
    let r = interp.attr_assign_ic(obj, name, val, Some(&cache));
    cell.store(cache.0.get(), Ordering::Relaxed);
    if let Err(e) = r {
        STATE.with(|s| s.borrow_mut().error = Some(e));
    }
}

/// `obj::Trait.attr`（field 52・1-4）。VM の `GetTraitAttr` と同じ `trait_access_evaled`。
extern "C" fn ar_get_trait_attr(
    obj_h: i64,
    trait_ptr: *const u8,
    trait_len: i32,
    name_ptr: *const u8,
    name_len: i32,
) -> i64 {
    let Some(trait_name) = native_str(trait_ptr, trait_len, "trait name") else { return TL_NONE };
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return TL_NONE };
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for get_trait_attr".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.trait_access_evaled(obj, trait_name, name) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// `obj::Trait.attr = v`（field 53・1-4）。VM の `SetTraitAttr` と同じ `trait_assign_evaled`。
extern "C" fn ar_assign_trait_attr(
    obj_h: i64,
    trait_ptr: *const u8,
    trait_len: i32,
    name_ptr: *const u8,
    name_len: i32,
    val_h: i64,
) {
    let Some(trait_name) = native_str(trait_ptr, trait_len, "trait name") else { return };
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return };
    let (has_err, obj, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_trait_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.trait_assign_evaled(obj, trait_name, name, val) {
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
    // ① 実体化してよい型（VM の `make_for_iterator` も実体化する型）は今までどおり要素を写す。
    let (has_err, obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let items: Option<Vec<Value>> = match &obj {
        Value::List(l) => Some(l.borrow().clone()),
        Value::FrozenList { state, layout } => {
            let fst = state.borrow();
            Some((0..fst.len).map(|i| layout.reconstruct_item(&fst.data, i)).collect())
        }
        Value::Tuple(t) => Some(t.all_values().to_vec()),
        Value::Str(s) => Some(s.chars().map(|c| Value::str(c.to_string())).collect()),
        Value::Dict(d) => Some(d.borrow().all_keys()),
        _ => None,
    };
    let entry = match items {
        Some(items) => NativeIter::Items(items, 0),
        // ② それ以外は VM と同じ反復プロトコル（`make_for_iterator` → `gen_next`・1-3）。
        //    ⚠ インタープリタへ呼び戻すので、`STATE` の借用を握ったまま呼ばない（`__iter__` がネイティブを呼ぶ）。
        None => {
            if interp_ptr.is_null() {
                STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for iter_from".to_string()));
                return TL_NONE;
            }
            let interp = unsafe { &mut *interp_ptr };
            match interp.make_for_iterator(obj) {
                Ok(Value::Generator(g)) => NativeIter::Gen(g),
                Ok(other) => {
                    let got = interp.type_name(&other);
                    STATE.with(|s| s.borrow_mut().error = Some(format!(
                        "TypeError: iterator protocol returned '{got}', not a generator"
                    )));
                    return TL_NONE;
                }
                Err(e) => {
                    STATE.with(|s| s.borrow_mut().error = Some(e));
                    return TL_NONE;
                }
            }
        }
    };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let idx = st.iter_table.len();
        st.iter_table.push(entry);
        -((idx as i64) + 2)
    })
}

/// `for` のループを抜けたときにイテレータを捨てる（field 71・11-1・D-12）。表を `iter_h` の位置まで縮めるので、
/// このループの中で作り、`return` などで捨て損ねたイテレータも一緒に捨てる（ループは入れ子なので、後ろにあるのは
/// 内側のものだけ）。
/// ⚠ 表に無いハンドルは黙って無視しない。`ar_iter_from` が失敗した（`TL_NONE` を返した）ときは、エラーが既に
///   入っているのでそのまま返す。
extern "C" fn ar_iter_drop(iter_h: i64) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let idx = (-(iter_h + 2)) as usize;
        if iter_h <= -2 && idx < st.iter_table.len() {
            st.iter_table.truncate(idx);
        } else if st.error.is_none() {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h} (iter_drop)"));
        }
    })
}

/// イテレータハンドルから次の要素ハンドルを取得する。
extern "C" fn ar_iter_next(iter_h: i64) -> i64 {
    enum Step { Value(Value), Stop, Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>) }
    let (step, interp_ptr) = STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() { return (Step::Stop, st.interp_ptr); }
        let ip = st.interp_ptr;
        if iter_h > -2 {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
            return (Step::Stop, ip);
        }
        let idx = (-(iter_h + 2)) as usize;
        let step = match st.iter_table.get_mut(idx) {
            Some(NativeIter::Items(items, pos)) => {
                if *pos < items.len() {
                    let v = items[*pos].clone();
                    *pos += 1;
                    Step::Value(v)
                } else {
                    Step::Stop
                }
            }
            Some(NativeIter::Gen(g)) => Step::Gen(g.clone()),
            // ⚠ 表に無いハンドル（最外の呼び出しを抜けて捨てた後など）を黙って「終わり」にしない
            None => {
                st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
                Step::Stop
            }
        };
        (step, ip)
    });
    match step {
        Step::Value(v) => push_handle(v),
        Step::Stop => TL_STOP_ITER,
        // ⚠ `gen_next` は本体（ジェネレータ・`__next__`）を走らせうるので、`STATE` の借用を落としてから呼ぶ。
        Step::Gen(g) => {
            if interp_ptr.is_null() {
                STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for iter_next".to_string()));
                return TL_STOP_ITER;
            }
            let interp = unsafe { &mut *interp_ptr };
            match interp.gen_next(&g) {
                Ok(Some(v)) => push_handle(v),
                Ok(None) => TL_STOP_ITER,
                Err(e) => {
                    STATE.with(|s| s.borrow_mut().error = Some(e));
                    TL_STOP_ITER
                }
            }
        }
    }
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
    // 残す値が無いとき生成コードは null を渡す（9-1）。null から slice は作れないので先に片づける
    if n <= 0 || handles_in.is_null() || handles_out.is_null() {
        STATE.with(|s| s.borrow_mut().arena.truncate(save as usize));
        return;
    }
    let n = n as usize;
    let ins = unsafe { std::slice::from_raw_parts(handles_in, n) };
    let outs = unsafe { std::slice::from_raw_parts_mut(handles_out, n) };
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        let vals: Vec<Value> = ins.iter().map(|&h| st.clone_value(h)).collect();
        st.arena.truncate(save as usize);
        for (i, val) in vals.into_iter().enumerate() {
            outs[i] = st.push_value(val);
        }
    });
}

// ── Typed value extraction ───────────────────────────────────────────────────

/// `let` 仮引数へ `mut` の値を渡すときの複製（field 54・native_object_access_plan.md 1-5）。
/// VM の `try_fast_bind` と同じ `copy_value`（`__copy__` があればそれ・無ければ凍結を解いた deep copy）。
/// ⚠ `ar_deep_copy`（`deep_copy_value`）は `let self` 用（VM も `self` には `__copy__` を呼ばない）。
extern "C" fn ar_copy_arg(h: i64) -> i64 {
    let (has_err, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for copy_arg".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.copy_value(val) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 宣言の複製（field 56・native_object_access_plan.md 1-6）。`mode` は `BindCopy` の符号（0 = `mut` の
/// 深いコピー・1 = `let` の複製＋フリーズ）で、VM の `StoreLocalDeepCopy` / `StoreLocalCopyFreeze` /
/// `StoreLocalFreezeInstance` と同じ `bind_copy` を呼ぶ。表に無い符号はエラー。
/// ⚠ 以前の生成コードは宣言で何も複製せず、`mut q = o.inner; q.v = 7` が `o.inner` を書き換えていた。
extern "C" fn ar_bind_value(mode: i32, h: i64) -> i64 {
    let (has_err, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let Some(mode) = crate::interpreter::BindCopy::from_code(mode) else {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: unknown binding mode {mode}")));
        return TL_NONE;
    };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for bind_value".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.bind_copy(mode, val) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// `let x = <名前>` で名前がこの関数のローカルでないとき（field 57・1-6）。ソースの可変性は値を引いた
/// `ar_get_global` と同じ探し方（`get_var`）で引き、VM の `StoreLocalFromIdent` と同じ `vm_let_value_from_ident`
/// で決める（`mut` → 複製＋フリーズ・`let` → そのまま・変数でない → 複製＋フリーズ）。
extern "C" fn ar_let_from_name(name_ptr: *const u8, name_len: i32, h: i64) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "variable name") else { return TL_NONE };
    let name = name.to_owned();
    let (has_err, val, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (binding from '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    let src_mutable = interp.get_var(&name).map(|v| v.is_mutable());
    match interp.vm_let_value_from_ident(src_mutable, val) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

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

/// field 32（以前に生成された `.arc` と中継コードが使う）。値渡しの複製はしない（引数の可変性が分からない）。
/// 今の生成コードは `ar_call_method_m`（field 55）を使う。
extern "C" fn ar_call_method(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
) -> i64 {
    call_method_common(obj_h, name_ptr, name_len, args_ptr, n_args, None)
}

/// 名前によるメソッド呼び出し＋値渡しの複製（field 55・native_object_access_plan.md 1-5・D-8）。
/// `mut_mask` は bit 0 = 受け手が `mut` の値か、bit i+1 = 引数 i が `mut` の値か（VM の `arg_is_mutable` と同じ規則で
/// 生成コードが決める）。`mut` の値を `let self` で受けるなら受け手を `deep_copy_value`、`let` 仮引数で受けるなら
/// 引数を `copy_value`（`__copy__` があればそれ）で複製してから呼ぶ（VM の `try_fast_bind` と同じ）。
/// ⚠ 以前はネイティブのメソッドへハンドルを素通しし、呼ばれた側の `let` が呼び出し元の後の変更で変わっていた。
extern "C" fn ar_call_method_m(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    call_method_common(obj_h, name_ptr, name_len, args_ptr, n_args, Some(mut_mask))
}

/// `mut_mask` の引数 `i`（bit i+1）が `mut` の値か。31 個目より後の引数はビットが無いので保守的に `mut`
/// （生成コードの `method_mut_mask` と同じ解釈）。
fn mask_arg_is_mut(mask: u32, i: usize) -> bool {
    i + 1 >= 32 || mask & (1 << (i + 1)) != 0
}

fn call_method_common(
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: Option<u32>,
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
        let native = super::lookup_native_method(&inst_rc.borrow().class.name, &name);
        let fn_ptr = native.map(|(ptr, _)| ptr);
        // 値渡しの複製（mask があるとき）に要る呼ばれた側の仮引数の可変性。オーバーロードがあれば
        // どれが呼ばれるか分からないので、ネイティブへ直行せずインタープリタの振り分けへ回す。
        let callee_params: Option<Vec<bool>> = match (mut_mask, fn_ptr) {
            (Some(_), Some(_)) => {
                let cls = inst_rc.borrow().class.clone();
                match cls.methods.get(&name) {
                    Some(ov) if ov.len() == 1 => Some(ov[0].params.iter().map(|p| p.mutable).collect()),
                    _ => None,
                }
            }
            _ => None,
        };
        // ⚠ 個数が宣言と合わない（既定値を省いた・1-11）呼び出しはインタープリタの振り分けへ回す（宣言どおりに束縛
        //   する）。生成した `_tl` は渡された個数を見ずに仮引数の数だけ読むので、直行すると引数の配列の外を読む。
        let arity_ok = {
            let cls = inst_rc.borrow().class.clone();
            match cls.methods.get(&name) {
                Some(ov) if ov.len() == 1 => ov[0].params.len() == n_args as usize + 1,
                _ => false,
            }
        };
        let direct = fn_ptr.filter(|_| arity_ok && (mut_mask.is_none() || callee_params.is_some()));
        if let Some(ptr) = direct {
            let mut all_args = Vec::with_capacity(1 + n_args as usize);
            // 複製を作る前のアリーナの長さ（1-15）。呼び出しの後でここまで巻き戻し、複製を呼び出しごとに捨てる。
            let mut copied_from: Option<u64> = None;
            let arena_len = || STATE.with(|s| s.borrow().arena.len() as u64);
            match (mut_mask, &callee_params) {
                (Some(mask), Some(pm)) => {
                    if interp_ptr.is_null() {
                        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
                        return TL_NONE;
                    }
                    let interp = unsafe { &mut *interp_ptr };
                    // 受け手（param 0 = self）
                    let self_let = pm.first().is_some_and(|m| !m);
                    if self_let && mask & 1 != 0 {
                        copied_from.get_or_insert_with(arena_len);
                        all_args.push(push_handle(crate::interpreter::Interpreter::deep_copy_value(obj.clone())));
                    } else {
                        all_args.push(obj_h);
                    }
                    for i in 0..n_args as usize {
                        let h = unsafe { *args_ptr.add(i) };
                        let param_let = pm.get(i + 1).is_some_and(|m| !m);
                        if param_let && mask_arg_is_mut(mask, i) {
                            copied_from.get_or_insert_with(arena_len);
                            match interp.copy_value(clone_value_at(h)) {
                                Ok(v) => all_args.push(push_handle(v)),
                                Err(e) => {
                                    STATE.with(|s| s.borrow_mut().error = Some(e));
                                    return TL_NONE;
                                }
                            }
                        } else {
                            all_args.push(h);
                        }
                    }
                }
                _ => {
                    all_args.push(obj_h);
                    for i in 0..n_args as usize {
                        all_args.push(unsafe { *args_ptr.add(i) });
                    }
                }
            }
            // メソッドはクラスを定義したモジュールの大域で、受け手のクラスの文脈で走る（1-7・1-9）
            if interp_ptr.is_null() {
                STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
                return TL_NONE;
            }
            let Some((_, globals)) = native else { unreachable!("`ptr` comes from `native`") };
            let class = inst_rc.borrow().class.clone();
            let prev = unsafe { &mut *interp_ptr }.enter_native_method(globals, class);
            let r = unsafe {
                let func: unsafe extern "C" fn(*const i64, i32) -> i64 = std::mem::transmute(ptr);
                func(all_args.as_ptr(), all_args.len() as i32)
            };
            unsafe { &mut *interp_ptr }.leave_native_method(prev);
            return match copied_from {
                Some(saved) => ar_arena_compact(r, saved),
                None => r,
            };
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
    // 引数の可変性: mask があればそれ（bit i+1）、無ければ（古い生成コード）保守的に `mut` とみなす。
    // インタープリタの束縛が `let` 仮引数 ← `mut` の値を複製する。
    let evaled: Vec<(Option<String>, Value, bool)> = args
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let is_mut = match mut_mask {
                Some(mask) => mask_arg_is_mut(mask, i),
                None => true,
            };
            (None, v, is_mut)
        })
        .collect();
    // インスタンスは VM の `CallMethod` と同じ入口へ、受け手の可変性（bit 0）を添えて渡す（6-1）。
    // `let self` は `mut` の受け手だけ複製する。mask の無い古い生成コードは保守的に `mut`。
    let r = if matches!(obj, Value::Instance(_)) {
        let recv_mut = mut_mask.is_none_or(|m| m & 1 != 0);
        interp.call_instance_method_evaled(obj, &name, evaled, None, None, recv_mut)
    } else {
        interp.eval_method_call_evaled(obj, &name, evaled)
    };
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

// ── Typed field-read callbacks ─────────────────────────────────────────────
//
// ⚠⚠ **読み出しの失敗を 0 に置き換えない**（native_object_access_plan.md 1-2）。以前は
//   `Ok(_) | Err(_) => 0.0` で、属性なし（`__init__` の中で初期化前のフィールドを読む等）・アクセス違反が
//   黙って 0 になっていた。エラーは `ar_get_attr` と同じく `STATE.error` に積む（呼び出しの出口で例外になる）。
//   数値への変換規則は `ar_to_float` / `ar_to_int` と同じ（`int` は広げる・`bool` は 0 / 1・ほかはエラー）。

/// 型付き読み出しの失敗を記録する（先に積まれたエラーがあればそちらを残す）。
fn set_field_read_error(msg: String) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_none() {
            st.error = Some(msg);
        }
    });
}

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
        Ok(Value::UInt(n)) => n as f64,
        Ok(Value::Bool(b)) => b as u8 as f64,
        Ok(other) => {
            let got = interp.type_name(&other);
            set_field_read_error(format!("TypeError: field '{name}' must be a number, not '{got}'"));
            0.0
        }
        Err(e) => {
            set_field_read_error(e);
            0.0
        }
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
        Ok(Value::UInt(n)) => n as i64,
        Ok(Value::Float(f)) => f as i64,
        Ok(Value::Bool(b)) => b as i64,
        Ok(other) => {
            let got = interp.type_name(&other);
            set_field_read_error(format!("TypeError: field '{name}' must be a number, not '{got}'"));
            0
        }
        Err(e) => {
            set_field_read_error(e);
            0
        }
    }
}

// ── IC 付きの属性読み出し（fields 58-60・native_object_access_plan.md 2-2・D-1） ──────────────────
//
// 生成コードは属性を読む箇所ごとに IC のセル（DLL 内の `u64`、初期値 0）を持ち、そのポインタを渡す。
// 命中判定は VM の `GetAttr` / `GetAttrLocal` と同じ `InstanceData::ic_slot`（アリーナ上のインスタンスを
// 借りたまま読み、`Value` を複製しない）、外れたら VM と同じミス経路 `get_attr_val(.., Some(cache))` が
// セルを埋める。⚠ 以前のネイティブの読み出し（fields 12 / 33 / 34）は IC を持たず、毎回 `get_attr_val` の
// 名前引き（`format!` と全フィールドの走査を含む）を通っていた。古い `.arc` のためにそちらも残す。

/// IC 付きの読み出しの結果。`Pending` は先に積まれたエラーがある（何もしない）。
enum IcRead {
    Value(Value, *mut crate::interpreter::Interpreter),
    Error(String),
    Pending,
}

/// IC 付きの読み出しの共通部。`cell` は非同期タスクのスレッドと共有されうるので `AtomicU64`（Relaxed）として
/// 読み書きする（class_id はプロセス全体で一意で、スレッドへの複製も同じ `field_index` を保つので命中した slot は正しい）。
fn read_attr_ic(obj_h: i64, name: &str, cell: *mut u64) -> IcRead {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() {
        return IcRead::Error(format!("NativeError: no inline cache cell for attribute '{name}'"));
    }
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let packed = cell.load(Ordering::Relaxed);
    let probe = STATE.with(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            return Err(None);
        }
        if obj_h >= 3 {
            if let Some(Value::Instance(rc)) = st.arena.get(obj_h as usize) {
                let inst = rc.borrow();
                if let Some((idx, _)) = inst.ic_slot(packed, true, || name) {
                    if let Some(v) = inst.field_value(idx) {
                        return Ok((v, st.interp_ptr));
                    }
                }
            }
        }
        Err(Some((st.clone_value(obj_h), st.interp_ptr)))
    });
    match probe {
        Ok((v, interp_ptr)) => IcRead::Value(v, interp_ptr),
        Err(None) => IcRead::Pending,
        Err(Some((obj, interp_ptr))) => {
            if interp_ptr.is_null() {
                return IcRead::Error(format!("NativeError: interpreter not set (get_attr '{name}')"));
            }
            let interp = unsafe { &mut *interp_ptr };
            let cache = crate::ast::AttrCache(std::cell::Cell::new(packed));
            let r = interp.get_attr_val(obj, name, Some(&cache));
            cell.store(cache.0.get(), Ordering::Relaxed);
            match r {
                Ok(v) => IcRead::Value(v, interp_ptr),
                Err(e) => IcRead::Error(e),
            }
        }
    }
}

/// `float` / `int` のフィールドで数値でない値が入っていたときの誤り（`get_float_field` / `get_int_field` と同じ文面）。
fn field_not_number(name: &str, got: &Value, interp_ptr: *mut crate::interpreter::Interpreter) {
    // `interp_ptr` は `read_attr_ic` が値と一緒に返したもの（命中経路でも空ではない: ネイティブ呼び出しの中でしか来ない）
    let got = if interp_ptr.is_null() { "unknown" } else { unsafe { &*interp_ptr }.type_name(got) };
    set_field_read_error(format!("TypeError: field '{name}' must be a number, not '{got}'"));
}

extern "C" fn ar_get_attr_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
    // 命中したら同じ借用のままアリーナへ積む（8-2）
    if let Some(h) = read_ic_hit(obj_h, cell, name_ptr, name_len, |st, v| Some(st.push_value(v))) {
        return h;
    }
    let Some(name) = site_name(name_ptr, name_len, cell, "attribute name") else { return TL_NONE };
    match read_attr_ic(obj_h, name, cell) {
        IcRead::Value(v, _) => push_handle(v),
        IcRead::Error(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
        IcRead::Pending => TL_NONE,
    }
}

/// 数値のフィールドを `float` として読むときの変換（`int` は広げる・`bool` は 0 / 1・ほかは `None` = エラー）。
/// 命中経路（`read_ic_hit`）と外れた道の両方で使う唯一の規則（native_object_access_plan.md 8-1）。
fn float_of(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(n) => Some(*n as f64),
        Value::UInt(n) => Some(*n as f64),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    }
}

/// [`float_of`] の `int` 版（`float` は切り捨て・`bool` は 0 / 1）。
fn int_of(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::UInt(n) => Some(*n as i64),
        Value::Float(f) => Some(*f as i64),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// 属性名をデバッグビルドの検査（`ic_slot` の slot と名前の突き合わせ）のためだけに読む。命中経路は名前を使わない。
fn debug_field_name<'a>(ptr: *const u8, len: i32) -> &'a str {
    if len <= 0 || ptr.is_null() {
        return "";
    }
    // SAFETY: `native_str` と同じ契約（ネイティブ側が `len` バイトの有効な領域を渡す）
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len as usize) }).unwrap_or("<invalid utf-8>")
}

/// IC 付きの読み出しの命中経路（native_object_access_plan.md 8-1・8-2・D-9）。IC が当たれば、アリーナとインスタンスを
/// 1 回借りたまま取り出したフィールドの値を `take` に渡し（数値へ変換する・アリーナへ積む）、その結果を返す。
/// それ以外（先に積まれたエラーがある・外れた・未初期化・`take` が受け取らない）は `None` で、呼ぶ側は今までの道
/// （`read_attr_ic`）へ回る（結果もエラーの文面も変わらない）。
/// ⚠ 属性名はここでは使わない（`ic_slot` のデバッグビルドの検査だけ）。
/// ⚠ 以前は毎回名前を検査し、値を `Value` → `Result` → `IcRead` に包んで返していた（数値の読み出し 1 回 28.8 ns のうち
///   17.8 ns・ハンドルを返す読み出しは約 20 ns。実験で切り分けた。IC の判定と取り出しは VM の `GetAttrLocal` と同じ関数）。
#[inline(always)]
fn read_ic_hit<T>(
    obj_h: i64,
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    take: impl FnOnce(&mut super::NativeCallState, Value) -> Option<T>,
) -> Option<T> {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() || obj_h < 3 {
        return None;
    }
    // SAFETY: `read_attr_ic` と同じ（生成コードが渡すのは DLL 内の `i64` の大域で、`AtomicU64` と同じ並び）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_some() {
            return None;
        }
        let v = {
            let Some(Value::Instance(rc)) = st.arena.get(obj_h as usize) else { return None };
            let inst = rc.borrow();
            let (idx, _) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len))?;
            inst.field_value(idx)?
        };
        take(&mut st, v)
    })
}

/// IC を持つ箇所の属性名（native_object_access_plan.md 8-2・D-9）。IC のセルが埋まっていれば、その箇所の名前は検査済みと
/// して検査を省く。セルは 0 から始まり、埋めるのは名前を `native_str` で検査した後の道（`read_attr_ic` の外れ・
/// `attr_assign_ic`）だけで、1 つのセルは 1 つの箇所・1 つの名前の定数にしか使わないため。空なら `native_str` で検査する。
/// ⚠ 以前は書き込み・ハンドルを返す読み出しのたびに UTF-8 として検査していた（1 回 4〜6 ns）。
fn site_name<'a>(name_ptr: *const u8, name_len: i32, cell: *mut u64, what: &str) -> Option<&'a str> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: `read_attr_ic` と同じ
    let filled = !cell.is_null() && unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed) != 0;
    if filled && name_len > 0 && !name_ptr.is_null() {
        // SAFETY: 長さとポインタは上で確かめた。中身はセルを埋める前に `native_str` が UTF-8 として検査した
        return Some(unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize)) });
    }
    native_str(name_ptr, name_len, what)
}

extern "C" fn ar_get_float_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> f64 {
    if let Some(f) = read_ic_hit(obj_h, cell, name_ptr, name_len, |_, v| float_of(&v)) {
        return f;
    }
    let Some(name) = site_name(name_ptr, name_len, cell, "field name") else { return 0.0 };
    match read_attr_ic(obj_h, name, cell) {
        IcRead::Value(v, interp_ptr) => match float_of(&v) {
            Some(f) => f,
            None => {
                field_not_number(name, &v, interp_ptr);
                0.0
            }
        },
        IcRead::Error(e) => {
            set_field_read_error(e);
            0.0
        }
        IcRead::Pending => 0.0,
    }
}

extern "C" fn ar_get_int_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
    if let Some(n) = read_ic_hit(obj_h, cell, name_ptr, name_len, |_, v| int_of(&v)) {
        return n;
    }
    let Some(name) = site_name(name_ptr, name_len, cell, "field name") else { return 0 };
    match read_attr_ic(obj_h, name, cell) {
        IcRead::Value(v, interp_ptr) => match int_of(&v) {
            Some(n) => n,
            None => {
                field_not_number(name, &v, interp_ptr);
                0
            }
        },
        IcRead::Error(e) => {
            set_field_read_error(e);
            0
        }
        IcRead::Pending => 0,
    }
}

// ── 生成コードの直接呼び出しの文脈（fields 62-63・native_object_access_plan.md 1-9） ──────────────
//
// 生成コードがモジュール内のメソッドを `_impl` で直接呼ぶとき、呼ばれるメソッドのクラスが呼ぶ側の文脈と違えば
// その前後で `current_class` を受け手のクラスにする（インタープリタのメソッド呼び出しと同じ）。
// 印は前の文脈そのもの（`Rc::into_raw`）で、`ar_leave_method_ctx` が受け取って戻す。生成コードは呼び出しと
// 呼び出しの間で抜けない（`raise` は呼ばれた側の関数から戻るだけ）ので、`ar_enter_method_ctx` と
// `ar_leave_method_ctx` は必ず対になる。
// ⚠ 先に積まれたエラーがあっても文脈は切り替える（対を崩さない）。

/// 印: 文脈を変えなかった（受け手がインスタンスでない。直接呼び出しは型検査の結果なので来ないが、
/// 来たら呼ばれた側の属性の読み書きがエラーにする）。
const CTX_UNCHANGED: i64 = -1;
/// 印: 前の文脈は無かった（`current_class` が `None`）。
const CTX_NONE: i64 = 0;

extern "C" fn ar_enter_method_ctx(obj_h: i64) -> i64 {
    let (obj, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.clone_value(obj_h), st.interp_ptr)
    });
    let Value::Instance(rc) = obj else { return CTX_UNCHANGED };
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for enter_method_ctx".to_string()));
        return CTX_UNCHANGED;
    }
    let interp = unsafe { &mut *interp_ptr };
    let class = rc.borrow().class.clone();
    match interp.current_class.replace(class) {
        None => CTX_NONE,
        Some(prev) => std::rc::Rc::into_raw(prev) as i64,
    }
}

extern "C" fn ar_leave_method_ctx(token: i64) {
    if token == CTX_UNCHANGED {
        return;
    }
    let interp_ptr = STATE.with(|s| s.borrow().interp_ptr);
    // SAFETY: `token` は `ar_enter_method_ctx` が `Rc::into_raw` で返したもの（0 は `None`）で、ここで 1 回だけ戻す。
    let prev = (token != CTX_NONE)
        .then(|| unsafe { std::rc::Rc::from_raw(token as *const crate::interpreter::ClassValue) });
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for leave_method_ctx".to_string()));
        return;
    }
    unsafe { &mut *interp_ptr }.current_class = prev;
}

/// 型検査の注釈で受け手のクラスを決めた直接呼び出しの前のガード（field 64・native_object_access_plan.md 3-2・D-2）。
///
/// 注釈はヒントであって意味論の根拠ではない（`list[T]` の要素・`new_type` など、実行時に保証されない型がある）
/// ので、受け手が本当にそのクラスのインスタンスか（class_id）と、`mut self` のメソッドなら不変インスタンスでないかを
/// 確かめる。外れたら 0 を返し、生成コードは名前による呼び出し（`ar_call_method_m`）へ回る（そちらが正しく
/// 振り分け、必要ならエラーを出す）。クラスの class_id は箇所ごとのセルに 1 回だけ名前で引いて覚える（0 = まだ）。
/// 名前で引けなければ（シャドウなど）0 のまま返す＝毎回名前による呼び出し（遅いが正しい）。
extern "C" fn ar_method_guard(obj_h: i64, class_ptr: *const u8, class_len: i32, cell: *mut u64, need_mutable: i32) -> i32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(class_name) = native_str(class_ptr, class_len, "class name") else { return 0 };
    if cell.is_null() || obj_h < 3 {
        return 0;
    }
    let probe = STATE.with(|s| {
        let st = s.borrow();
        match st.arena.get(obj_h as usize) {
            Some(Value::Instance(rc)) => {
                let inst = rc.borrow();
                Some((inst.class.class_id, inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0, st.interp_ptr))
            }
            _ => None,
        }
    });
    let Some((class_id, immutable, interp_ptr)) = probe else { return 0 };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let mut want = cell.load(Ordering::Relaxed);
    if want == 0 {
        if interp_ptr.is_null() {
            return 0;
        }
        let interp = unsafe { &*interp_ptr };
        match interp.get_var(class_name).map(|v| v.get_value()) {
            Some(Value::Class(cls)) => {
                want = cls.class_id as u64;
                cell.store(want, Ordering::Relaxed);
            }
            _ => return 0,
        }
    }
    i32::from(class_id as u64 == want && !(need_mutable != 0 && immutable))
}

/// モジュール内のクラスの生成（field 67・native_object_access_plan.md 5-1・D-4）。VM の `Call`（呼び先がクラス）と
/// 同じ `instantiate_evaled` を直接呼ぶ。引数の可変性（`let` 仮引数の複製）は `mut_mask` から VM と同じ規則で渡す。
/// ⚠ 以前の生成コードはクラスを `CB_GET_GLOBAL` で値として引き、`CB_CALL_FN` の遅い道（`call_value_with_args`）で
///   呼んでいた（呼び出しのたびにコールバック 2 回と値の振り分け）。
/// クラスは名前で引く（`CB_GET_GLOBAL` と同じ `get_var`。1-7 でモジュールの大域にいる）。クラスでない値に
/// 束縛されていたら（型検査が通さないはずだが）値として呼ぶ（`call_value_with_args`）。
extern "C" fn ar_construct(name_ptr: *const u8, name_len: i32, args_ptr: *const i64, n_args: i32, mut_mask: u32) -> i64 {
    construct_by_name(std::ptr::null_mut(), name_ptr, name_len, args_ptr, n_args, mut_mask)
}

/// `Class(args)` の呼び出し箇所ごとのセルつき版（field 70・native_object_access_plan.md 10-2・D-11）。セルに前回の
/// クラスの `class_id + 1` が入っていて、インタープリタの表（`native_classes`）にあれば、名前の検査も名前で引くことも
/// しない。無ければ（最初の 1 回・ワーカーの最初の 1 回）`ar_construct` と同じく名前で引き、表とセルを埋める。
/// ⚠ 以前（field 67）は生成のたびに名前を UTF-8 として検査し、`get_var` で引いていた（1 回 約 80 ns）。
/// 同じ箇所の名前はいつも同じクラスを指す（モジュールの最上位のクラスは束縛し直せない・ローカルに同名があれば
/// 生成コードはこの道を使わない）。
extern "C" fn ar_construct_ic(
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if !cell.is_null() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
        if packed != 0 {
            let (has_err, interp_ptr) = STATE.with(|s| {
                let st = s.borrow();
                (st.error.is_some(), st.interp_ptr)
            });
            if has_err {
                return TL_NONE;
            }
            if !interp_ptr.is_null() {
                let interp = unsafe { &mut *interp_ptr };
                if let Some(cls) = interp.native_classes.get(&((packed - 1) as u32)).cloned() {
                    return construct_class(interp, cls, args_ptr, n_args, mut_mask);
                }
            }
        }
    }
    construct_by_name(cell, name_ptr, name_len, args_ptr, n_args, mut_mask)
}

/// クラスを名前で引いて生成する（field 67 と field 70 の外れた道）。`cell` があればクラスを表とセルに入れる。
fn construct_by_name(
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(name) = native_str(name_ptr, name_len, "class name") else { return TL_NONE };
    let (has_err, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (constructing '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_var(name).map(|v| v.get_value()) {
        Some(Value::Class(cls)) => {
            if !cell.is_null() {
                interp.native_classes.insert(cls.class_id, cls.clone());
                // SAFETY: 上と同じ
                unsafe { &*(cell as *const AtomicU64) }.store(u64::from(cls.class_id) + 1, Ordering::Relaxed);
            }
            construct_class(interp, cls, args_ptr, n_args, mut_mask)
        }
        Some(other) => {
            let args: Vec<Value> = (0..n_args as usize).map(|i| clone_value_at(unsafe { *args_ptr.add(i) })).collect();
            finish_construct(interp.call_value_with_args(other, args))
        }
        None => finish_construct(Err(format!("NameError: '{name}' is not defined"))),
    }
}

/// クラスを VM の `Call` と同じ `instantiate_evaled` で生成する。引数の可変性は `mut_mask` から VM と同じ規則で渡す。
fn construct_class(
    interp: &mut crate::interpreter::Interpreter,
    cls: std::rc::Rc<crate::interpreter::ClassValue>,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
) -> i64 {
    let evaled = (0..n_args as usize)
        .map(|i| (None, clone_value_at(unsafe { *args_ptr.add(i) }), mask_arg_is_mut(mut_mask, i)))
        .collect();
    finish_construct(interp.instantiate_evaled(cls, evaled))
}

fn finish_construct(r: Result<Value, String>) -> i64 {
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// `len(x)`（field 68・native_object_access_plan.md 5-2）。組み込みの `len` と同じ `len_of`（唯一の実装）を呼び、
/// 結果を生の int で返す。⚠ 以前の生成コードは `len` を `CB_GET_GLOBAL` で値として引き、`CB_CALL_FN` で呼んで
/// ハンドルを受け、`CB_TO_INT` で開けていた。
extern "C" fn ar_len(h: i64) -> i64 {
    let (has_err, v, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return 0; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for len".to_string()));
        return 0;
    }
    match unsafe { &mut *interp_ptr }.len_of(v) {
        Ok(Value::Int(n)) => n,
        Ok(other) => {
            STATE.with(|s| s.borrow_mut().error = Some(format!("TypeError: len() returned a non-int value: {other:?}")));
            0
        }
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            0
        }
    }
}

/// `str(<int>)`（field 69・5-2）。組み込みの `str` と同じ `call_type_by_name_evaled("str", ..)` を、値を箱に入れずに呼ぶ。
extern "C" fn ar_str_of_int(n: i64) -> i64 {
    let (has_err, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for str".to_string()));
        return TL_NONE;
    }
    match unsafe { &mut *interp_ptr }.call_type_by_name_evaled("str", vec![Value::Int(n)]) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
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

/// 組み込み関数を名前で呼ぶ（field 50・native_object_access_plan.md 1-3）。本体は VM の `CallBuiltin` と同じ
/// `Interpreter::call_builtin_by_name_evaled`（同名のグローバル束縛があればそちらを呼ぶ）。
/// ⚠ 以前は `get_global` で組み込み関数を値として引いており、値として登録されていない
///   `range` / `print` / `repr` / `enumerate` / `zip` / `getenv` がコンパイル済みコードで `NameError` になっていた。
extern "C" fn ar_call_builtin(name_ptr: *const u8, name_len: i32, args_ptr: *const i64, n_args: i32) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "builtin name") else { return TL_NONE };
    let (has_err, interp_ptr) = STATE.with(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        STATE.with(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (calling '{name}')")));
        return TL_NONE;
    }
    let args: Vec<Value> = (0..n_args.max(0) as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_builtin_by_name_evaled(name, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            STATE.with(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
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
    call_builtin: ar_call_builtin,
    assign_attr: ar_assign_attr,
    get_trait_attr: ar_get_trait_attr,
    assign_trait_attr: ar_assign_trait_attr,
    copy_arg: ar_copy_arg,
    call_method_m: ar_call_method_m,
    bind_value: ar_bind_value,
    let_from_name: ar_let_from_name,
    get_attr_ic: ar_get_attr_ic,
    get_float_field_ic: ar_get_float_field_ic,
    get_int_field_ic: ar_get_int_field_ic,
    assign_attr_ic: ar_assign_attr_ic,
    enter_method_ctx: ar_enter_method_ctx,
    leave_method_ctx: ar_leave_method_ctx,
    method_guard: ar_method_guard,
    assign_int_field_ic: ar_assign_int_field_ic,
    assign_float_field_ic: ar_assign_float_field_ic,
    construct: ar_construct,
    len: ar_len,
    str_of_int: ar_str_of_int,
    construct_ic: ar_construct_ic,
    iter_drop: ar_iter_drop,
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

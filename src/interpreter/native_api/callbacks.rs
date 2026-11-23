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
    with_state(|s| {
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
    with_state(|s| s.borrow_mut().error = Some(msg));
}

extern "C" fn ar_make_str(ptr: *const u8, len: i32) -> i64 {
    let Some(text) = native_str(ptr, len, "string") else { return TL_NONE };
    with_state(|s| s.borrow_mut().push_value(Value::str(text)))
}

/// 文字列のリテラルを呼び出し箇所ごとに 1 回だけ作る（field 72・11-2・D-12）。
///
/// `cell`（DLL の中の箇所ごとの `u64`・プロセスに 1 つ）には、初めて評価したときにプロセスで一意の番号を入れる。
/// 文字列はスレッドの状態の表 `str_consts` のその番号の位置に置き、以後は参照数の加算で積む（VM の定数表と同じ）。
/// 別のスレッド（非同期タスクのワーカー）は同じ番号で自分の表に作る（`Rc<str>` はスレッドをまたげない）。
/// ⚠ 中身の検査（UTF-8）はスレッドごとに初めて作るときだけ（中身は箇所ごとに変わらない）。
extern "C" fn ar_make_str_ic(cell: *mut u64, ptr: *const u8, len: i32) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    if cell.is_null() {
        return ar_make_str(ptr, len);
    }
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let mut id = cell.load(Ordering::Relaxed);
    if id == 0 {
        let fresh = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        // 別のスレッドが先に入れたらその番号を使う（どちらの番号でも同じ文字列になる）
        id = match cell.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => fresh,
            Err(cur) => cur,
        };
    }
    let idx = (id - 1) as usize;
    let hit = with_state(|s| {
        let mut st = s.borrow_mut();
        let v = st.str_consts.get(idx).and_then(|v| v.clone())?;
        Some(st.push_value(v))
    });
    if let Some(h) = hit {
        return h;
    }
    let Some(text) = native_str(ptr, len, "string") else { return TL_NONE };
    let v = Value::str(text);
    with_state(|s| {
        let mut st = s.borrow_mut();
        if st.str_consts.len() <= idx {
            st.str_consts.resize(idx + 1, None);
        }
        st.str_consts[idx] = Some(v.clone());
        st.push_value(v)
    })
}

// Build list/tuple/dict in one borrow: clone all items then push the container.
extern "C" fn ar_make_list(items_ptr: *const i64, n: i32) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let items: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let list = Rc::new(RefCell::new(items));
        st.push_value(Value::List(list))
    })
}

extern "C" fn ar_make_tuple(items_ptr: *const i64, n: i32) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let elements: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let tuple = Rc::new(TupleData::new(elements));
        st.push_value(Value::Tuple(tuple))
    })
}

extern "C" fn ar_make_dict(keys_ptr: *const i64, vals_ptr: *const i64, n: i32) -> i64 {
    with_state(|s| {
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
    let (v, interp_ptr) = with_state(|s| {
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
            with_state(|s| {
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
    let (has_err, lhs, rhs, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(a), st.clone_value(b), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let ast_op = match i32_to_binop(op) {
        Some(o) => o,
        None => {
            with_state(|s| s.borrow_mut().error = Some(format!("NativeError: invalid binop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for binop".to_string()));
        return TL_NONE;
    }
    // VM の `BinOp` と同じ `apply_binop_dyn`（`__add__` / `__eq__` などを呼ぶ・`in` の `__eq__`・1-14）。
    // ⚠ 以前は `apply_binop` で演算子のメソッドを見ず、`a + b` が TypeError、`a == b` が `__eq__` でなく構造の比較になっていた。
    let interp = unsafe { &mut *interp_ptr };
    match interp.apply_binop_dyn(&ast_op, lhs, rhs) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_unop(op: i32, a: i64) -> i64 {
    let (has_err, operand, interp_ptr) = with_state(|s| {
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
            with_state(|s| s.borrow_mut().error = Some(format!("NativeError: invalid unop code {op}")));
            return TL_NONE;
        }
    };
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for unop".to_string()));
        return TL_NONE;
    }
    // VM の単項演算と同じ `apply_unary_dyn`（`__neg__` などを呼ぶ・1-14）。⚠ 以前は `apply_unary` で見ていなかった。
    let interp = unsafe { &mut *interp_ptr };
    match interp.apply_unary_dyn(&ast_op, operand) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    with_state(|s| {
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
        let vals: Vec<Value> = with_state(|s| {
            let st = s.borrow();
            (0..n).map(|i| st.clone_value(unsafe { *args_ptr.add(i) })).collect()
        });
        match ta.marshal(&vals, &sig.params, &[]) {
            Ok(Marshalled::Ready) => true,
            Ok(Marshalled::TypeMismatch) => false,
            Err(e) => {
                with_state(|s| s.borrow_mut().error = Some(e));
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
            with_state(|s| s.borrow_mut().error = Some(e));
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
    let (has_err, fn_val, interp_ptr) = with_state(|s| {
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
                                with_state(|s| s.borrow_mut().error = Some(fn_ref.missing_symbol_error(&e)));
                                abort_native_call(is_outermost);
                                return TL_NONE;
                            }
                        }
                    }
                    None => {
                        with_state(|s| s.borrow_mut().error = Some(format!(
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
                    copied_from.get_or_insert_with(|| with_state(|s| s.borrow().arena.len() as u64));
                    match unsafe { &mut *interp_ptr }.copy_value(clone_value_at(*h)) {
                        Ok(v) => *h = push_handle(v),
                        Err(e) => {
                            with_state(|s| s.borrow_mut().error = Some(e));
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

            if with_state(|s| s.borrow().error.is_some()) {
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
        with_state(|s| s.borrow_mut().error = Some(
            "NativeError: interpreter not set for call_fn".to_string()
        ));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_value_with_args(fn_val, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_attr(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return TL_NONE };
    let name = name.to_owned();
    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for get_attr".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_attr_val(obj, &name, None) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    let (has_err, obj, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for set_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.set_attr_val(obj, &name, val) {
        with_state(|s| s.borrow_mut().error = Some(e));
    }
}

/// コンパイル済み Arrow コードの `obj.attr = v`（field 51・native_object_access_plan.md 1-4）。
/// VM の `SetAttr` と同じ `attr_assign_evaled`。⚠ 以前は `set_attr_val` を通っていたので、
/// `static mut` へのインスタンス経由の代入（`self.total = ..`）が「has no field」で落ちていた。
extern "C" fn ar_assign_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
    let Some(name) = native_str(name_ptr, name_len, "attribute name") else { return };
    let (has_err, obj, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.attr_assign_evaled(obj, name, val) {
        with_state(|s| s.borrow_mut().error = Some(e));
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
    if assign_number_ic_hit(obj_h, name_ptr, name_len, Value::Int(v), cell) {
        return;
    }
    assign_attr_ic_value(obj_h, name_ptr, name_len, Value::Int(v), cell);
}

/// `obj.attr = <float>`（IC 付き・field 66・4-2）。
extern "C" fn ar_assign_float_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, v: f64, cell: *mut u64) {
    if assign_number_ic_hit(obj_h, name_ptr, name_len, Value::Float(v), cell) {
        return;
    }
    assign_attr_ic_value(obj_h, name_ptr, name_len, Value::Float(v), cell);
}

/// 数値の書き込みの IC の命中経路（copy_alloc_speed_plan.md 8-8）。書き込みの IC が当たれば、アリーナとインスタンスを 1 回
/// 借りたまま検査と格納を済ませて `true`。当たらなければ何もせずに `false`（呼ぶ側が今までの道 `assign_attr_ic_value` へ回る）。
///
/// やることは `attr_assign_inner` の命中の枝と同じ（`ic_slot` → `field_mutability_vec` → [`Interpreter::store_field_checked`]。
/// 不変のフィールド・不変のインスタンス・型の合わない値は同じ文面のエラーになる）。違うのは通り道だけ:
/// - 受け手をアリーナから写さない（`Rc` の増減が無い）・インタープリタを取り出さない・一時の `AttrCache` を作らない
/// - 格納の前の複製（`copy_for_store`）を呼ばない: 値は `int` / `float` で、複製しても同じ値
/// ⚠ 以前は当たっても `attr_assign_ic` を通り、`b.mass = 2.0` が 1 回 42 ns かかっていた（読み出しは 12.5 ns・§0.7）。
/// ⚠ 値がハンドルの書き込み（field 61）はここを通さない: 格納の前の複製にインタープリタが要る。
fn assign_number_ic_hit(obj_h: i64, name_ptr: *const u8, name_len: i32, val: Value, cell: *mut u64) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    debug_assert!(matches!(val, Value::Int(_) | Value::Float(_)));
    if cell.is_null() || obj_h < 3 || name_len <= 0 || name_ptr.is_null() {
        return false;
    }
    // SAFETY: `read_ic_hit` と同じ（生成コードが渡すのは DLL 内の `i64` の大域）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    if packed == 0 {
        return false;
    }
    with_state(|s| {
        let mut st = s.borrow_mut();
        // 先に積まれたエラーがあれば何もしない（今までの道も、セルが埋まっていればここで戻る）
        if st.error.is_some() {
            return true;
        }
        let stored = {
            let Some(Value::Instance(rc)) = st.arena.get(obj_h as usize) else { return false };
            // 借りられない（誰かが借りたまま）なら今までの道に任せる
            let Ok(mut inst) = rc.try_borrow_mut() else { return false };
            let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len)) else {
                return false;
            };
            let is_mutable = inst.class.field_mutability_vec.get(idx).copied().unwrap_or(true);
            // 複製の診断（`copy_stats`）は今までの道と同じ数え方にする（格納の入口を 1 回・写していない）
            crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::Store, &val, true);
            // SAFETY: セルが埋まっている箇所の名前は、埋める前に `native_str` が UTF-8 として検査した（`site_name` と同じ）
            let name = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
            };
            crate::interpreter::Interpreter::store_field_checked(&mut inst, idx, name, val, is_mutable)
        };
        if let Err(e) = stored {
            st.error = Some(e);
        }
        true
    })
}

/// `a.b.c = v` を 1 回で行う（field 74・12-3・D-13）。途中の段は VM の `GetAttr` と同じ IC で読み（命中なら
/// `ic_slot`、外れたら `get_attr_val(.., Some(cache))` がセルを埋める）、最後の段は fields 61 / 65 / 66 と同じ
/// `attr_assign_ic` で書く。途中の値はアリーナに積まない（積んで周ごとに巻き戻していた分を省く）。
/// 属性名はセルが空のときだけ検査する（`site_name`）。
extern "C" fn ar_assign_path_ic(root_h: i64, steps: *const i64, n_steps: i32, kind: i32, value: i64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if n_steps < 2 || steps.is_null() {
        set_native_error(format!("NativeError: invalid attribute path from native code ({n_steps} step(s))"));
        return;
    }
    // SAFETY: 生成コードは段ごとに 3 つの `i64`（名前のポインタ・長さ・セルのポインタ）を並べて渡す
    let table = unsafe { std::slice::from_raw_parts(steps, 3 * n_steps as usize) };
    let (has_err, root, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(root_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        set_native_error("NativeError: interpreter not set for assign_path".to_string());
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    let val = match kind {
        0 => clone_value_at(value),
        1 => Value::Int(value),
        2 => Value::Float(f64::from_bits(value as u64)),
        _ => {
            set_native_error(format!("NativeError: unknown value kind {kind} (assign_path)"));
            return;
        }
    };
    let step = |k: usize| (table[3 * k] as *const u8, table[3 * k + 1] as i32, table[3 * k + 2] as *mut u64);
    let mut obj = root;
    let last = n_steps as usize - 1;
    for k in 0..last {
        let (np, nl, cell) = step(k);
        let Some(name) = site_name(np, nl, cell, "attribute name") else { return };
        if cell.is_null() {
            set_native_error(format!("NativeError: no inline cache cell for attribute '{name}'"));
            return;
        }
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
        let cell = unsafe { &*(cell as *const AtomicU64) };
        let packed = cell.load(Ordering::Relaxed);
        let hit = match &obj {
            Value::Instance(rc) => {
                let inst = rc.borrow();
                inst.ic_slot(packed, true, || name).and_then(|(idx, _)| inst.field_value(idx))
            }
            _ => None,
        };
        obj = match hit {
            Some(v) => v,
            None => {
                let cache = crate::ast::AttrCache(std::cell::Cell::new(packed));
                let r = interp.get_attr_val(obj, name, Some(&cache));
                cell.store(cache.0.get(), Ordering::Relaxed);
                match r {
                    Ok(v) => v,
                    Err(e) => {
                        with_state(|s| s.borrow_mut().error = Some(e));
                        return;
                    }
                }
            }
        };
    }
    let (np, nl, cell) = step(last);
    let Some(name) = site_name(np, nl, cell, "attribute name") else { return };
    if cell.is_null() {
        set_native_error(format!("NativeError: no inline cache cell for attribute '{name}'"));
        return;
    }
    // SAFETY: 上と同じ
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let cache = crate::ast::AttrCache(std::cell::Cell::new(cell.load(Ordering::Relaxed)));
    let r = interp.attr_assign_ic(obj, name, val, Some(&cache));
    cell.store(cache.0.get(), Ordering::Relaxed);
    if let Err(e) = r {
        with_state(|s| s.borrow_mut().error = Some(e));
    }
}

/// fields 61 / 65 / 66 の本体（値の受け取り方だけが違う）。
fn assign_attr_ic_value(obj_h: i64, name_ptr: *const u8, name_len: i32, val: Value, cell: *mut u64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(name) = site_name(name_ptr, name_len, cell, "attribute name") else { return };
    if cell.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: no inline cache cell for attribute '{name}'")));
        return;
    }
    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    let cache = crate::ast::AttrCache(std::cell::Cell::new(cell.load(Ordering::Relaxed)));
    let r = interp.attr_assign_ic(obj, name, val, Some(&cache));
    cell.store(cache.0.get(), Ordering::Relaxed);
    if let Err(e) = r {
        with_state(|s| s.borrow_mut().error = Some(e));
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
    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for get_trait_attr".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.trait_access_evaled(obj, trait_name, name) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    let (has_err, obj, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(val_h), st.interp_ptr)
    });
    if has_err { return; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for assign_trait_attr".to_string()));
        return;
    }
    let interp = unsafe { &mut *interp_ptr };
    if let Err(e) = interp.trait_assign_evaled(obj, trait_name, name, val) {
        with_state(|s| s.borrow_mut().error = Some(e));
    }
}

extern "C" fn ar_subscript(obj_h: i64, key_h: i64) -> i64 {
    let (has_err, obj, key, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.clone_value(key_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for subscript".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.eval_subscript(obj, key) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_get_global(name_ptr: *const u8, name_len: i32) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "global name") else { return TL_NONE };
    let name = name.to_owned();
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (looking up '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_val(&name) {
        Some(v) => push_handle(v),
        None => {
            with_state(|s| s.borrow_mut().error = Some(format!("NameError: '{name}' is not defined")));
            TL_NONE
        }
    }
}

/// イテラブルなオブジェクトハンドルからイテレータハンドルを生成する。
/// イテレータハンドルは `-(idx+2)` の負値でエンコードされる。
extern "C" fn ar_iter_from(obj_h: i64) -> i64 {
    let (has_err, obj) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h))
    });
    if has_err { return TL_NONE; }
    let entry = match snapshot_items(&obj) {
        Some(items) => NativeIter::Items(items, 0),
        None => match protocol_iter(obj, "iter_from") {
            Some(g) => NativeIter::Gen(g),
            None => return TL_NONE,
        },
    };
    push_iter(entry)
}

/// ① 実体化してよい型（VM の `make_for_iterator` も実体化する型）の要素の写し。ほかの型は `None`
/// （[`protocol_iter`] で 1 つずつ取る）。反復の途中のリストの変更は反復に効かない（VM と同じ）。
fn snapshot_items(obj: &Value) -> Option<Vec<Value>> {
    match obj {
        Value::List(l) => Some(l.borrow().clone()),
        Value::FrozenList { state, layout } => {
            let fst = state.borrow();
            Some((0..fst.len).map(|i| layout.reconstruct_item(&fst.data, i)).collect())
        }
        Value::Tuple(t) => Some(t.all_values().to_vec()),
        Value::Str(s) => Some(s.chars().map(|c| Value::str(c.to_string())).collect()),
        Value::Dict(d) => Some(d.borrow().all_keys()),
        _ => None,
    }
}

/// ② 写さない型は VM と同じ反復プロトコル（`make_for_iterator` → `gen_next`・1-3）。失敗したらエラーを積んで `None`。
/// ⚠ インタープリタへ呼び戻すので、`STATE` の借用を握ったまま呼ばない（`__iter__` がネイティブを呼ぶ）。
fn protocol_iter(obj: Value, what: &str) -> Option<std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>> {
    let interp_ptr = with_state(|s| s.borrow().interp_ptr);
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for {what}")));
        return None;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.make_for_iterator(obj) {
        Ok(Value::Generator(g)) => Some(g),
        Ok(other) => {
            let got = interp.type_name(&other);
            with_state(|s| s.borrow_mut().error = Some(format!(
                "TypeError: iterator protocol returned '{got}', not a generator"
            )));
            None
        }
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            None
        }
    }
}

/// イテレータの表へ足し、ハンドル `-(idx+2)` を返す。
fn push_iter(entry: NativeIter) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let idx = st.iter_table.len();
        st.iter_table.push(entry);
        -((idx as i64) + 2)
    })
}

/// [`ar_iter_begin`] がアリーナを伸ばすとき、要素の分に足す欄の数（ループの本体が 1 周に積む一時値の分）。
const RUN_SLACK: usize = 1024;

/// field 87（copy_alloc_speed_plan.md 7-20）: `for` の入口。[`ar_iter_from`] と同じくイテレータのハンドルを返し、写す型なら
/// 要素を**全部アリーナへ積んで**、そのハンドルの並びの番地を `out[0]`・個数を `out[1]` へ書く。生成コードは周ごとに並びを
/// 読むだけで、要素を取るコールバックを呼ばない。写さない型は `out[1] = -1`（field 84 で 1 つずつ）。
/// 失敗したら `TL_NONE`（エラーは積んである）と 0 個の並び（生成コードのループは 1 周もしない）。
/// - 要素は `iter_from` の写しと同じ値（リストの要素の参照を 1 つずつ足す）で、ハンドルは `push_value` と同じ（`None`・
///   真偽値・小さい整数は決まったハンドル）。積み方は [`push_run`]（7-21）。⚠ 積んだ欄はループの保存点より下にある
///   （生成コードは保存点をこの呼び出しの後に取る）ので、周ごとの巻き戻しでは消えない。消えるのはループの外の巻き戻し
///   （呼ぶ側の生成コード・呼び出しの出口）
/// - 並びは表の `NativeIter::Run` が持ち、`iter_drop` まで書き換えない（表が伸びても並びの番地は動かない）
/// - ⚠ 以前（7-15）は入口で要素を `Vec` へ写し、周ごとに `iter_next_at` が写しから取り出して積んでいた（1 周に 1 回の
///   コールバック・k1 の約 2 割）
extern "C" fn ar_iter_begin(obj_h: i64, out: *mut i64) -> i64 {
    // SAFETY: 生成コードは 2 個の `i64` の置き場を渡す
    let out = unsafe { std::slice::from_raw_parts_mut(out, 2) };
    out[0] = 0;
    out[1] = 0;
    let (has_err, obj) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h))
    });
    if has_err { return TL_NONE; }
    let hs = match &obj {
        // リストは写しの `Vec` を作らずに要素の参照を足して積む（ほかの型は `iter_from` と同じ写しから）
        Value::List(l) => {
            let l = l.borrow();
            with_state(|s| push_run(&mut s.borrow_mut(), l.len(), l.iter(), write_clone))
        }
        _ => match snapshot_items(&obj) {
            Some(items) => with_state(|s| {
                push_run(&mut s.borrow_mut(), items.len(), items.into_iter(), |v, slot| {
                    slot.write(v);
                })
            }),
            None => {
                return match protocol_iter(obj, "iter_begin") {
                    Some(g) => {
                        out[1] = -1;
                        push_iter(NativeIter::Gen(g))
                    }
                    None => TL_NONE,
                };
            }
        },
    };
    // 並びの番地は `Vec` を表へ移しても変わらない（中身の確保は動かない）
    out[0] = hs.as_ptr() as i64;
    out[1] = hs.len() as i64;
    push_iter(NativeIter::Run(hs))
}

/// [`ar_iter_begin`] の並びを作る: `n` 個の要素 `items` を積み、ハンドルの並びを返す（ハンドルは `push_value` と同じ・
/// [`fixed_handle`]）。要素はアリーナの空き欄へ直接書く（copy_alloc_speed_plan.md 7-21）。
/// - アリーナは 1 回だけ、ちょうど要る大きさ（と本体の一時値の分）に伸ばす。⚠ 1 つずつ積むと倍々に伸び、伸ばす途中の
///   確保と余りの分だけ最大の量が増えた（10 万要素の k5 で、写しの `Vec` だけだった以前より 約 6 MB・7-20）
/// - ⚠ `n` は `items` の個数（空き欄をこの数で確保する。超えて空き欄が尽きたら添字の検査で止まる）。`write` は
///   アリーナに触らないこと（空き欄を借りている）
/// - ⚠ 以前（7-20）は `&mut dyn Iterator` から `push_value` で 1 つずつ積んでいた。`next()` がメモリ経由で返した要素を
///   16 バイトずつ読み直す命令（要素の種類の 1 バイトとポインタの 8 バイトを別々に書いた直後なので、書いた値を読みへ
///   直接渡せない）で待ち、k1 の 18%（ほぼ全部がその 1 命令）を占めていた
#[inline(always)]
fn push_run<T: std::borrow::Borrow<Value>>(
    st: &mut NativeCallState,
    n: usize,
    items: impl Iterator<Item = T>,
    write: impl Fn(T, &mut std::mem::MaybeUninit<Value>),
) -> Vec<i64> {
    if st.arena.capacity() - st.arena.len() < n + RUN_SLACK {
        st.arena.reserve_exact(n + RUN_SLACK);
    }
    let base = st.arena.len();
    let spare = st.arena.spare_capacity_mut();
    let mut k = 0usize;
    let hs: Vec<i64> = items
        .map(|v| match fixed_handle(v.borrow()) {
            Some(h) => h,
            None => {
                write(v, &mut spare[k]);
                k += 1;
                (base + k - 1) as i64
            }
        })
        .collect();
    // SAFETY: 空き欄の先頭 `k` 個は上で書いた（途中で止まったら書いた値は捨てられずに残るだけで、未初期化は読まない）
    unsafe { st.arena.set_len(base + k) };
    hs
}

/// リストの要素の複製を空き欄 `slot` へ書く（[`ar_iter_begin`]）。よく出る種類はその場で複製して直接書く
/// （同じ値・`Value::clone` と同じ意味）。
/// ⚠ 複製を一時の値にしてから写すと、LLVM が一時の置き場へ 1〜8 バイトずつ書いて 16 バイトで読み直す形にし、
///   7-20 と同じ待ちが出た（試作で k1 の 11% がその 1 命令）。`Value::clone`（derive）も結果をメモリ経由で返す。
#[inline(always)]
fn write_clone(v: &Value, slot: &mut std::mem::MaybeUninit<Value>) {
    match v {
        Value::Instance(rc) => {
            slot.write(Value::Instance(rc.clone()));
        }
        Value::Str(s) => {
            slot.write(Value::Str(s.clone()));
        }
        Value::Float(f) => {
            slot.write(Value::Float(*f));
        }
        Value::Int(n) => {
            slot.write(Value::Int(*n));
        }
        _ => {
            slot.write(v.clone());
        }
    }
}

/// `for` のループを抜けたときにイテレータを捨てる（field 71・11-1・D-12）。表を `iter_h` の位置まで縮めるので、
/// このループの中で作り、`return` などで捨て損ねたイテレータも一緒に捨てる（ループは入れ子なので、後ろにあるのは
/// 内側のものだけ）。
/// ⚠ 表に無いハンドルは黙って無視しない。`ar_iter_from` が失敗した（`TL_NONE` を返した）ときは、エラーが既に
///   入っているのでそのまま返す。
extern "C" fn ar_iter_drop(iter_h: i64) {
    with_state(|s| {
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
    iter_next_impl(iter_h, None)
}

/// field 84（copy_alloc_speed_plan.md 7-15）: アリーナを保存点 `save`（0 以上のとき）まで巻き戻してから次の要素。
/// 周の終わりに残す値が無いループの巻き戻しを、次の周の頭のこの呼び出しにまとめる。
extern "C" fn ar_iter_next_at(iter_h: i64, save: i64) -> i64 {
    iter_next_impl(iter_h, usize::try_from(save).ok())
}

/// [`ar_iter_next`] / [`ar_iter_next_at`] の本体。リストなどの写し（`NativeIter::Items`）の要素は、複製せずに写しから
/// **取り出して**積む（写しの要素は 1 回しか使わない・7-15）。巻き戻し・取り出し・積むのを 1 回の借用で行う。
/// ⚠ 以前は要素を複製して積み（写しにも残る）、`push_handle` でもう 1 回状態を借りていた。要素はインスタンスの `Rc` で、
///   参照の数はばらばらの塊にあるので、触るたびに離れたメモリを読む（`Value::clone` が k2 の自己時間の 14%）。
/// ⚠ 巻き戻しはエラーがあってもする（周の終わりの `compact_many` と同じ）。
fn iter_next_impl(iter_h: i64, rewind: Option<usize>) -> i64 {
    enum Step { Done(i64), Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>) }
    enum Next { Val(Value), Stop, Gen(std::rc::Rc<RefCell<crate::interpreter::GeneratorState>>), Bad }
    let (step, interp_ptr) = with_state(|s| {
        let mut st = s.borrow_mut();
        if let Some(len) = rewind {
            st.arena.truncate(len);
        }
        if st.error.is_some() { return (Step::Done(TL_STOP_ITER), st.interp_ptr); }
        let ip = st.interp_ptr;
        if iter_h > -2 {
            st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
            return (Step::Done(TL_STOP_ITER), ip);
        }
        let idx = (-(iter_h + 2)) as usize;
        let next = match st.iter_table.get_mut(idx) {
            Some(NativeIter::Items(items, pos)) => match items.get_mut(*pos) {
                Some(slot) => {
                    *pos += 1;
                    Next::Val(std::mem::replace(slot, Value::None))
                }
                None => Next::Stop,
            },
            Some(NativeIter::Gen(g)) => Next::Gen(g.clone()),
            // 並び（`ar_iter_begin`）は生成コードが番地から読む。ここで進めると生成コードの添字とずれるので止める
            Some(NativeIter::Run(_)) => {
                st.error = Some(format!("NativeError: iter handle {iter_h} is a handle run (iter_begin); read it directly"));
                return (Step::Done(TL_STOP_ITER), ip);
            }
            None => Next::Bad,
        };
        let step = match next {
            Next::Val(v) => Step::Done(st.push_value(v)),
            Next::Stop => Step::Done(TL_STOP_ITER),
            Next::Gen(g) => Step::Gen(g),
            // ⚠ 表に無いハンドル（最外の呼び出しを抜けて捨てた後など）を黙って「終わり」にしない
            Next::Bad => {
                st.error = Some(format!("NativeError: invalid iter handle {iter_h}"));
                Step::Done(TL_STOP_ITER)
            }
        };
        (step, ip)
    });
    match step {
        Step::Done(h) => h,
        // ⚠ `gen_next` は本体（ジェネレータ・`__next__`）を走らせうるので、`STATE` の借用を落としてから呼ぶ。
        Step::Gen(g) => {
            if interp_ptr.is_null() {
                with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for iter_next".to_string()));
                return TL_STOP_ITER;
            }
            let interp = unsafe { &mut *interp_ptr };
            match interp.gen_next(&g) {
                Ok(Some(v)) => push_handle(v),
                Ok(None) => TL_STOP_ITER,
                Err(e) => {
                    with_state(|s| s.borrow_mut().error = Some(e));
                    TL_STOP_ITER
                }
            }
        }
    }
}

extern "C" fn ar_is_type(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "type name") else { return TL_NONE };
    let (obj, interp_ptr) = with_state(|s| {
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
    with_state(|s| s.borrow().arena.len() as u64)
}

extern "C" fn ar_arena_compact(h: i64, saved: u64) -> i64 {
    let saved = saved as usize;
    // Special handles (None/True/False): just truncate — no arena slot.
    if h < 3 {
        with_state(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    if (h as usize) < saved {
        // Handle lives before the save point — still valid after truncation.
        with_state(|s| s.borrow_mut().arena.truncate(saved));
        return h;
    }
    // Handle was allocated inside the callee: clone, truncate, re-push — all in one borrow.
    with_state(|s| {
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
        with_state(|s| s.borrow_mut().arena.truncate(save as usize));
        return;
    }
    let n = n as usize;
    let ins = unsafe { std::slice::from_raw_parts(handles_in, n) };
    let outs = unsafe { std::slice::from_raw_parts_mut(handles_out, n) };
    with_state(|s| {
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
    let (has_err, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for copy_arg".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.copy_value(val) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 宣言の複製（field 56・native_object_access_plan.md 1-6）。`mode` は `BindCopy` の符号（0 = `mut` の
/// 深いコピー・1 = `let` の複製＋フリーズ）で、VM の `StoreLocalDeepCopy` / `StoreLocalCopyFreeze` /
/// `StoreLocalFreezeInstance` と同じ `bind_copy` を呼ぶ。表に無い符号はエラー。
/// ⚠ 以前の生成コードは宣言で何も複製せず、`mut q = o.inner; q.v = 7` が `o.inner` を書き換えていた。
///
/// `mode` の [`crate::interpreter::BindCopy::FRESH_BIT`] の bit は「`h` はコールバックが作りたての値を積んだばかりの欄」（生成コードが付ける・12-1）。
/// そのときだけ、誰とも共有していなければ複製しない（`bind_copy_held`。数えない参照はその欄の 1 つ）。
/// ⚠ 付いていないときは必ず複製する: ローカル変数のハンドルは番号で同じ欄を指し、参照を数えないので、
///   `if` 式などが変数の欄をそのまま返すと共有を見抜けない。
extern "C" fn ar_bind_value(mode: i32, h: i64) -> i64 {
    let (has_err, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    let fresh_bit = crate::interpreter::BindCopy::FRESH_BIT;
    let held = (mode & fresh_bit != 0).then_some(1);
    let mode = mode & !fresh_bit;
    let Some(mode) = crate::interpreter::BindCopy::from_code(mode) else {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: unknown binding mode {mode}")));
        return TL_NONE;
    };
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for bind_value".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.bind_copy_held(mode, val, held) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    let (has_err, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (binding from '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    let src_mutable = interp.get_var(&name).map(|v| v.is_mutable());
    match interp.vm_let_value_from_ident(src_mutable, val) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_deep_copy(h: i64) -> i64 {
    let val = clone_value_at(h);
    crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &val, false);
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
            let r = with_state(|s| match s.borrow().arena.get(n as usize) {
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
            let r = with_state(|s| match s.borrow().arena.get(n as usize) {
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
    with_state(|s| s.borrow().error.is_some() as i32)
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
        Ok(Some(buf)) => with_state(|s_tls| {
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
            with_state(|s| s.borrow_mut().protocol_args.push((h, arg)));
        }
        Err(e) => set_argument_error(e),
    }
}

/// `ArPtrProtocol*` の引数の呼び出し後（ハンドル経路・4-4）。C が `address` を 0 に書き戻していたら、その `Ptr` を使えなくする
/// （解放関数はもう呼ばない・D-3）。
extern "C" fn ar_protocol_done(h: i64, address_after: u64) {
    // `protocol_arg` が組み立てた引数を探して、型つき経路と同じ後処理（`PtrProtocolArg::finish`）をする:
    // `Ptr` を無効にする・`list` へ書き戻す（4-7）
    let arg = with_state(|s| {
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
    with_state(|s| {
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
        n => with_state(|s| {
            s.borrow().arena.get(n as usize).map_or("invalid handle".to_string(), |v| runtime_type_name(v).to_string())
        }),
    }
}

/// 引数の変換の誤りを設定する。⚠ **最初の誤りを残す**（2 つ目の引数の誤りで 1 つ目を上書きしない）。
fn set_argument_error(msg: String) {
    with_state(|s| {
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
    with_state(|s_tls| {
        let mut st = s_tls.borrow_mut();
        let mut bytes = s.into_bytes();
        bytes.push(0u8);
        st.cstr_bufs.push(bytes);
        st.cstr_bufs.last().unwrap().as_ptr()
    })
}

extern "C" fn ar_write_handle(target_h: i64, new_val_h: i64) {
    if target_h < 3 { return; }
    with_state(|s| {
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
    let (has_err, item, maybe_list) = with_state(|s| {
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
    let (type_name, msg) = with_state(|s| {
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
    with_state(|s| s.borrow_mut().pending_raise = Some((type_name, msg)));
    TL_EXCEPTION
}

extern "C" fn ar_make_cell(init_h: i64) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(init_h);
        let cell = Rc::new(RefCell::new(vec![val]));
        st.push_value(Value::List(cell))
    })
}

extern "C" fn ar_get_cell(cell_h: i64) -> i64 {
    // Clone the Rc, release borrow, then access cell outside STATE lock.
    let maybe_list = with_state(|s| {
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
    let (val, maybe_list) = with_state(|s| {
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
    let Some(name) = native_str(name_ptr, name_len, "method name") else { return TL_NONE };
    call_method_common(obj_h, name, args_ptr, n_args, None, None, 0)
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
    let Some(name) = native_str(name_ptr, name_len, "method name") else { return TL_NONE };
    call_method_common(obj_h, name, args_ptr, n_args, Some(mut_mask), None, 0)
}

/// 名前によるメソッド呼び出しの、箇所ごとのセルと FFI の検査の情報つきの版（field 73・11-3・D-12）。
///
/// VM の `CallMethodLocal` と同じく、名前は `&str` のまま振り分けへ渡し（検査はセルが空のときだけ・8-2 の `site_name`）、
/// 組み込み型の受け手は `vm_method_call_other` と同じ手順（外部言語なら戻り値を宣言型と照合）で呼ぶ。
/// セルの値は「名前を検査した」印（1）だけ。
extern "C" fn ar_call_method_ic(
    cell: *mut u64,
    obj_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
    info_ptr: *const u8,
    info_len: i32,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let fresh = take_fresh_mask();
    let Some(name) = site_name(name_ptr, name_len, cell, "method name") else { return TL_NONE };
    if !cell.is_null() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
        unsafe { &*(cell as *const AtomicU64) }.store(1, Ordering::Relaxed);
    }
    let info = if info_ptr.is_null() {
        None
    } else {
        let Some(info) = native_str(info_ptr, info_len, "call info") else { return TL_NONE };
        Some(info)
    };
    call_method_common(obj_h, name, args_ptr, n_args, Some(mut_mask), info, fresh)
}

/// 生成コードが埋め込んだ FFI の検査の情報（`表示名 \x1f 宣言型 \x1f 位置`）で戻り値を照合する（VM の
/// `check_ffi_return` と同じ検査・同じ文言）。宣言型はコンパイルのときに `InferredType::from_ann` で読み戻せることを
/// 確かめてある（読み戻せない型の呼び出しを含む関数はコンパイルしない）ので、読めなければ壊れた `.arc` としてエラーにする。
fn check_native_ffi_return(
    interp: &mut crate::interpreter::Interpreter,
    lang: &str,
    value: Value,
    info: &str,
) -> Result<Value, String> {
    let mut parts = info.splitn(3, '\x1f');
    let (Some(name), Some(decl), Some(where_)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("NativeError: malformed call info from native code: {info:?}"));
    };
    let Some(declared) = crate::type_check::InferredType::from_ann(decl) else {
        return Err(format!("NativeError: the declared type `{decl}` of `{name}` from native code cannot be read"));
    };
    interp.check_ffi_return_declared(lang, value, &declared, name, &|| where_.to_string())
}

/// `mut_mask` の引数 `i`（bit i+1）が `mut` の値か。31 個目より後の引数はビットが無いので保守的に `mut`
/// （生成コードの `method_mut_mask` と同じ解釈）。
fn mask_arg_is_mut(mask: u32, i: usize) -> bool {
    i + 1 >= 32 || mask & (1 << (i + 1)) != 0
}

/// 名前によるメソッド呼び出しの共通部。`name` は生成コードの定数（呼び出しの間は有効）なので複製しない（11-3）。
/// `ffi` は FFI の検査の情報（field 73 だけが渡す・`check_native_ffi_return`）。
fn call_method_common(
    obj_h: i64,
    name: &str,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: Option<u32>,
    ffi: Option<&str>,
    fresh: u32,
) -> i64 {

    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }

    // Native dispatch: check NATIVE_METHODS table first.
    if let Value::Instance(ref inst_rc) = obj {
        let native = super::lookup_native_method(inst_rc.borrow().class.class_id, name);
        let fn_ptr = native.map(|(ptr, _)| ptr);
        // 値渡しの複製（mask があるとき）に要る呼ばれた側の仮引数の可変性。オーバーロードがあれば
        // どれが呼ばれるか分からないので、ネイティブへ直行せずインタープリタの振り分けへ回す。
        let callee_params: Option<Vec<bool>> = match (mut_mask, fn_ptr) {
            (Some(_), Some(_)) => {
                let cls = inst_rc.borrow().class.clone();
                match cls.methods.get(name) {
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
            match cls.methods.get(name) {
                Some(ov) if ov.len() == 1 => ov[0].params.len() == n_args as usize + 1,
                _ => false,
            }
        };
        let direct = fn_ptr.filter(|_| arity_ok && (mut_mask.is_none() || callee_params.is_some()));
        if let Some(ptr) = direct {
            let mut all_args = Vec::with_capacity(1 + n_args as usize);
            // 複製を作る前のアリーナの長さ（1-15）。呼び出しの後でここまで巻き戻し、複製を呼び出しごとに捨てる。
            let mut copied_from: Option<u64> = None;
            let arena_len = || with_state(|s| s.borrow().arena.len() as u64);
            match (mut_mask, &callee_params) {
                (Some(mask), Some(pm)) => {
                    if interp_ptr.is_null() {
                        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
                        return TL_NONE;
                    }
                    let interp = unsafe { &mut *interp_ptr };
                    // 受け手（param 0 = self）
                    let self_let = pm.first().is_some_and(|m| !m);
                    // 読むだけのメソッドへは複製せずに渡す（4-4・VM と同じ判定）。引くのは複製が起きるときだけ
                    let wants_copy = (self_let && mask & 1 != 0)
                        || (0..n_args as usize).any(|i| pm.get(i + 1).is_some_and(|m| !m) && mask_arg_is_mut(mask, i));
                    let borrow = wants_copy && interp.may_borrow_for(name);
                    if self_let && mask & 1 != 0 && borrow {
                        all_args.push(obj_h);
                    } else if self_let && mask & 1 != 0 {
                        copied_from.get_or_insert_with(arena_len);
                        crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &obj, false);
                        all_args.push(push_handle(crate::interpreter::Interpreter::deep_copy_value(obj.clone())));
                    } else {
                        all_args.push(obj_h);
                    }
                    for i in 0..n_args as usize {
                        let h = unsafe { *args_ptr.add(i) };
                        let param_let = pm.get(i + 1).is_some_and(|m| !m);
                        if param_let && mask_arg_is_mut(mask, i) && borrow && !interp.has_copy_hook(&clone_value_at(h)) {
                            all_args.push(h);
                        } else if param_let && mask_arg_is_mut(mask, i) {
                            copied_from.get_or_insert_with(arena_len);
                            // 作りたての印の付いた引数は、欄の 1 個を数えずに共有を判定する（2-4。写さなければ同じ欄を渡す）
                            let is_fresh = i < 31 && fresh & (1 << (i + 1)) != 0 && h >= super::INT_CACHE_BASE as i64;
                            let val = clone_value_at(h);
                            let copied = if is_fresh {
                                interp.copy_value_held(val.clone(), 2).map(|v| {
                                    if crate::interpreter::Interpreter::same_heap_value(&v, &val) { h } else { push_handle(v) }
                                })
                            } else {
                                interp.copy_value(val).map(push_handle)
                            };
                            match copied {
                                Ok(nh) => all_args.push(nh),
                                Err(e) => {
                                    with_state(|s| s.borrow_mut().error = Some(e));
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
                with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
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
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set for call_method '{name}'")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    // 引数の可変性: mask があればそれ（bit i+1）、無ければ（古い生成コード）保守的に `mut` とみなす。
    // インタープリタの束縛が `let` 仮引数 ← `mut` の値を複製する。
    // 引数は 1 つの `Vec` に直接（11-3・以前は値の `Vec` を作ってから詰め直していた）
    let evaled: Vec<(Option<String>, Value, bool)> = (0..n_args as usize)
        .map(|i| {
            let is_mut = match mut_mask {
                Some(mask) => mask_arg_is_mut(mask, i),
                None => true,
            };
            (None, arg_value(args_ptr, i, fresh), is_mut)
        })
        .collect();
    // インスタンスは VM の `CallMethod` と同じ入口へ、受け手の可変性（bit 0）を添えて渡す（6-1）。
    // `let self` は `mut` の受け手だけ複製する。mask の無い古い生成コードは保守的に `mut`。
    let r = if matches!(obj, Value::Instance(_)) {
        let recv_mut = mut_mask.is_none_or(|m| m & 1 != 0);
        interp.call_instance_method_evaled(obj, name, evaled, None, None, recv_mut)
    } else {
        // VM の `vm_method_call_other` と同じ手順: 呼ぶ前に外部言語かを見て、戻り値を宣言型と照合する（11-3）。
        // 情報が無い（型検査が戻り値の型を付けていない・古い `.arc` の field 55）ときは VM の未採番と同じく照合しない
        let lang = crate::interpreter::Interpreter::foreign_call_lang(&obj, name);
        let r = interp.eval_method_call_evaled(obj, name, evaled);
        match (r, lang, ffi) {
            (Ok(v), Some(l), Some(info)) => check_native_ffi_return(interp, l, v, info),
            (r, _, _) => r,
        }
    };
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    with_state(|s| {
        let mut st = s.borrow_mut();
        if st.error.is_none() {
            st.error = Some(msg);
        }
    });
}

extern "C" fn ar_get_float_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> f64 {
    let Some(name) = native_str(name_ptr, name_len, "field name") else { return 0.0 };
    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0.0; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_float_field '{name}')")));
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
    let (has_err, obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(obj_h), st.interp_ptr)
    });
    if has_err { return 0; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (get_int_field '{name}')")));
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
    let probe = with_state(|s| {
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
            with_state(|s| s.borrow_mut().error = Some(e));
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
    with_state(|s| {
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
    // private / protected のメンバーを持つクラスが 1 つも無い間は、文脈を読む者がいないので切り替えない（7-3）
    if !crate::interpreter::value::restricted_members_exist() {
        return CTX_UNCHANGED;
    }
    let (obj, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.clone_value(obj_h), st.interp_ptr)
    });
    let Value::Instance(rc) = obj else { return CTX_UNCHANGED };
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for enter_method_ctx".to_string()));
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
    let interp_ptr = with_state(|s| s.borrow().interp_ptr);
    // SAFETY: `token` は `ar_enter_method_ctx` が `Rc::into_raw` で返したもの（0 は `None`）で、ここで 1 回だけ戻す。
    let prev = (token != CTX_NONE)
        .then(|| unsafe { std::rc::Rc::from_raw(token as *const crate::interpreter::ClassValue) });
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for leave_method_ctx".to_string()));
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
    // ⚠ クラスの名前（の UTF-8 の検査）はセルが空のときだけ使う（copy_alloc_speed_plan.md 7-9）。以前は毎回検査していて、
    //   それがコンパイル済みの k1 の 9%（このガードの 2/3）だった。セルを埋めるのは名前が正しいときだけなので、正しくない
    //   名前は今までどおり、名前を引くたびにエラーになる。
    if cell.is_null() || obj_h < 3 {
        return 0;
    }
    let probe = with_state(|s| {
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
        let Some(class_name) = native_str(class_ptr, class_len, "class name") else { return 0 };
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
    construct_by_name(std::ptr::null_mut(), name_ptr, name_len, args_ptr, n_args, mut_mask, 0)
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
    // 作りたての印を読んで 0 に戻すのと、エラーの有無・インタープリタを読むのを、状態を 1 回借りて済ませる（7-10）
    let (fresh, has_err, interp_ptr) = with_state(|s| {
        let mut st = s.borrow_mut();
        (std::mem::take(&mut st.fresh_mask), st.error.is_some(), st.interp_ptr)
    });
    if !cell.is_null() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
        if packed != 0 {
            if has_err {
                return TL_NONE;
            }
            if !interp_ptr.is_null() {
                let interp = unsafe { &mut *interp_ptr };
                if let Some(cls) = interp.native_classes.get(&((packed - 1) as u32)).cloned() {
                    return construct_class(interp, cls, args_ptr, n_args, mut_mask, fresh);
                }
            }
        }
    }
    construct_by_name(cell, name_ptr, name_len, args_ptr, n_args, mut_mask, fresh)
}

/// クラスを名前で引いて生成する（field 67 と field 70 の外れた道）。`cell` があればクラスを表とセルに入れる。
/// 直後の生成・名前によるメソッド呼び出しの「作りたての引数」の印を入れる（field 75・2-4）。
extern "C" fn ar_set_fresh(mask: u32) {
    with_state(|s| s.borrow_mut().fresh_mask = mask);
}

/// 印を読んで 0 に戻す。⚠ `ar_construct_ic` / `ar_call_method_ic` は**入口の最初に**読む（途中で抜けても、次の
/// 呼び出しに印が残らないように。`ar_construct_ic` はエラーの有無と一緒に、状態を借りる 1 回の中で読む・7-10）。
fn take_fresh_mask() -> u32 {
    with_state(|s| std::mem::take(&mut s.borrow_mut().fresh_mask))
}

/// 引数 `i` の値。作りたての印（`fresh` の bit i+1）が付いていれば、アリーナの欄から**取り出す**（欄は `None` になる。
/// 生成コードはその欄を二度と読まない）。付いていなければ今までどおり欄の値の写し。
/// ⚠ 小さい整数・真偽値・`None` の共有の欄（`INT_CACHE_BASE` より前）は取り出さない。
fn arg_value(args_ptr: *const i64, i: usize, fresh: u32) -> Value {
    with_state(|s| arg_value_in(&mut s.borrow_mut(), args_ptr, i, fresh))
}

/// [`arg_value`] の本体。スレッドの状態を借りたまま引数を続けて読む所（生成・7-10）が使う。
fn arg_value_in(st: &mut super::NativeCallState, args_ptr: *const i64, i: usize, fresh: u32) -> Value {
    let h = unsafe { *args_ptr.add(i) };
    if i < 31 && fresh & (1 << (i + 1)) != 0 && h >= super::INT_CACHE_BASE as i64 {
        return match st.arena.get_mut(h as usize) {
            Some(slot) => std::mem::replace(slot, Value::None),
            None => Value::None,
        };
    }
    st.clone_value(h)
}

/// 直接呼び出しの `let self` / `let` 仮引数へ渡す作りたての値（field 76・2-4）。`h` の欄の 1 個は数えずに、
/// 誰とも共有していなければ複製しない（VM の `copy_value_held` / 受け手の `deep_copy_value` と同じ結果）。
extern "C" fn ar_copy_fresh(h: i64, is_self: i32) -> i64 {
    let (has_err, val, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for copy_fresh".to_string()));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    if is_self != 0 {
        // `let self`: `deep_copy_value`（`__copy__` を呼ばない・印を保つ）。共有していなければそのまま
        let unshared = h >= super::INT_CACHE_BASE as i64 && crate::interpreter::Interpreter::is_unshared(&val, 2);
        crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &val, unshared);
        if unshared {
            return h;
        }
        return push_handle(crate::interpreter::Interpreter::deep_copy_value(val));
    }
    let held = usize::from(h >= super::INT_CACHE_BASE as i64);
    match interp.copy_value_held(val.clone(), held + 1) {
        // 写さなかった（同じ実体）なら欄をそのまま使う
        Ok(v) if crate::interpreter::Interpreter::same_heap_value(&v, &val) => h,
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 直接呼び出しの `let self` / `let` 仮引数へ渡す `mut` の値（field 77・4-4）。呼ぶ関数が読むだけで、走っている
/// タスクが無ければ複製しない（VM の `try_fast_bind` と同じ判定・`Interpreter::may_borrow_for`）。
/// ⚠ セルはプロセスに 1 つだが、覚えるのは「名前が読むだけの集合にあるか」だけ。非同期タスクのワーカーは
///   自分が走っているタスクなので、セルが 1 でも必ず複製する（`no_live_tasks` が偽）。
///
/// `is_self` は種類: 0 = `let` 仮引数・1 = `let self`・2 = `let` 仮引数で、渡す値が受け手・ほかの引数と**重ならない**と
/// 生成コードが言えたもの（同じ変数の別のフィールド・4-6）。2 のときは、呼ぶ関数が自分の `mut` 仮引数だけを書き換える
/// 関数（`ReadonlyFns::is_confined_writer`）でも複製しない。
extern "C" fn ar_borrow_or_copy(h: i64, is_self: i32, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if !cell.is_null() && h >= super::INT_CACHE_BASE as i64 && crate::interpreter::async_mgr::no_live_tasks() {
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        let cell = unsafe { &*(cell as *const AtomicU64) };
        let mut known = cell.load(Ordering::Relaxed);
        if known == 0 {
            let interp_ptr = with_state(|s| s.borrow().interp_ptr);
            if let (Some(name), false) = (native_str(name_ptr, name_len, "function name"), interp_ptr.is_null()) {
                let interp = unsafe { &*interp_ptr };
                let ok = if is_self == 2 {
                    interp.readonly_fns.is_confined_writer(name)
                } else {
                    interp.readonly_fns.contains(name)
                };
                known = if ok { 1 } else { 2 };
                cell.store(known, Ordering::Relaxed);
            }
        }
        if known == 1 {
            if is_self == 1 {
                return h;
            }
            // 仮引数: `__copy__` を持つクラスの値は今までどおり `__copy__` を呼ぶ
            let (val, interp_ptr) = with_state(|s| {
                let st = s.borrow();
                (st.clone_value(h), st.interp_ptr)
            });
            if !interp_ptr.is_null() && !unsafe { &*interp_ptr }.has_copy_hook(&val) {
                return h;
            }
        }
    }
    if is_self == 1 {
        ar_deep_copy(h)
    } else {
        ar_copy_arg(h)
    }
}

fn construct_by_name(
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    args_ptr: *const i64,
    n_args: i32,
    mut_mask: u32,
    fresh: u32,
) -> i64 {
    let Some(name) = native_str(name_ptr, name_len, "class name") else { return TL_NONE };
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (constructing '{name}')")));
        return TL_NONE;
    }
    let interp = unsafe { &mut *interp_ptr };
    match interp.get_var(name).map(|v| v.get_value()) {
        Some(Value::Class(cls)) => {
            remember_site_class(interp, cell, &cls);
            construct_class(interp, cls, args_ptr, n_args, mut_mask, fresh)
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
    fresh: u32,
) -> i64 {
    // 作りたての印の付いた引数は欄から取り出す（2-4・参照の数を VM と同じにして、束縛と格納の複製を省く）。
    // 並びは置き場から取り（`instantiate_evaled` が戻す）、引数は状態を 1 回借りて全部読む（7-10）
    let mut evaled = interp.take_evaled_buf();
    with_state(|s| {
        let mut st = s.borrow_mut();
        evaled.extend(
            (0..n_args.max(0) as usize).map(|i| (None, arg_value_in(&mut st, args_ptr, i, fresh), mask_arg_is_mut(mut_mask, i))),
        );
    });
    finish_construct(interp.instantiate_evaled(cls, evaled))
}

fn finish_construct(r: Result<Value, String>) -> i64 {
    match r {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

/// 生成の箇所のセルとインタープリタの表に、名前で引いたクラスを覚える（2 回目からは名前で引かない・10-2）。
fn remember_site_class(
    interp: &mut crate::interpreter::Interpreter,
    cell: *mut u64,
    cls: &std::rc::Rc<crate::interpreter::ClassValue>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if !cell.is_null() {
        interp.native_classes.insert(cls.class_id, cls.clone());
        // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
        unsafe { &*(cell as *const AtomicU64) }.store(u64::from(cls.class_id) + 1, Ordering::Relaxed);
    }
}

/// 生成の箇所 `cell`（null でないこと）のクラス。`ar_construct_ic` と同じ引き方: セルが埋まっていて表にあればそれ、
/// 無ければ名前で引いて覚える。クラスでない値・見つからない名前は `None`（呼ぶ側は今までの道へ回り、そちらがエラーを出す）。
fn site_class(
    interp: &mut crate::interpreter::Interpreter,
    cell: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
) -> Option<std::rc::Rc<crate::interpreter::ClassValue>> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    if packed != 0 {
        if let Some(cls) = interp.native_classes.get(&((packed - 1) as u32)) {
            return Some(cls.clone());
        }
    }
    let name = native_str(name_ptr, name_len, "class name")?;
    match interp.get_var(name).map(|v| v.get_value()) {
        Some(Value::Class(cls)) => {
            remember_site_class(interp, cell, &cls);
            Some(cls)
        }
        _ => None,
    }
}

/// 合成した `__init__` で作るインスタンスを、引数を格納する前の状態で作ってアリーナに積む（field 83・
/// copy_alloc_speed_plan.md 7-11）。生成コードはこのあと数値の引数を番地 `*out` へ直接書き、初期化の印を立てる。これは
/// `store_auto_init` が `store_field_checked` で行う格納と同じ結果になる: 作ったばかりのインスタンスは不変でなく、どの slot も
/// まだ初期化されていないので格納の検査は通り、値の変換（幅への切り詰め・`float32` への丸め）は生成コードが
/// `InstanceData::store_field` と同じ規則で行う。
/// クラスは `ar_construct_ic` と同じセル・同じ引き方（[`site_class`]）で決め、生成コードの表（`DirectClassDesc`）のとおりか
/// （並び・幅・`mut`・全部 public）と、合成した `__init__` の格納の順が `slots` と同じか（`Interpreter::auto_init_direct_ok`）を
/// 確かめる。確かめたクラスは照合のセル `checked` に番号 + 1 で覚え、2 回目からは照合しない（答えはクラスと、箇所ごとに
/// 決まった表と順だけで決まる。`ar_inst_block` のセルと同じ考え）。外れ・先に積まれたエラーは 0 を返して何もしない
/// （生成コードは今までの `ar_construct_ic` へ回り、結果もエラーも今までどおり）。
#[allow(clippy::too_many_arguments)]
extern "C" fn ar_new_instance(
    cell: *mut u64,
    checked: *mut u64,
    name_ptr: *const u8,
    name_len: i32,
    desc: *const crate::interpreter::value::DirectClassDesc,
    slots: *const i64,
    n: i32,
    out: *mut *mut u64,
) -> i64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() || checked.is_null() || desc.is_null() || out.is_null() || n <= 0 || slots.is_null() {
        return 0;
    }
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err || interp_ptr.is_null() {
        return 0;
    }
    let interp = unsafe { &mut *interp_ptr };
    let Some(cls) = site_class(interp, cell, name_ptr, name_len) else { return 0 };
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（IC のセルと同じ）
    let checked = unsafe { &*(checked as *const AtomicU64) };
    let id1 = u64::from(cls.class_id) + 1;
    if checked.load(Ordering::Relaxed) != id1 {
        // SAFETY: `desc` は生成コードが DLL の中に置いた表
        let Some((cname, fields)) = (unsafe { read_direct_desc(desc) }) else { return 0 };
        // SAFETY: 生成コードが渡す `n` 個の `i64` の定数（DLL の中）
        let want = unsafe { std::slice::from_raw_parts(slots, n as usize) };
        if !cls.matches_direct_desc(cname, &fields) || !interp.auto_init_direct_ok(&cls, want) {
            return 0;
        }
        checked.store(id1, Ordering::Relaxed);
    }
    let rc = std::rc::Rc::new(std::cell::RefCell::new(crate::interpreter::Interpreter::new_instance_data(&cls)));
    // SAFETY: 塊（`Box<[u64]>`）はインスタンスが生きている間動かない。インスタンスはすぐ下でアリーナの欄に置き、生成コードは
    // 次にコールバックを呼ぶ前に書き終える（その間インスタンスを借りている者はいない・D-13 ④）。
    let block = unsafe { (*rc.as_ptr()).raw.as_mut_ptr() };
    let h = push_handle(Value::Instance(rc));
    unsafe { *out = block };
    h
}

/// `len(x)`（field 68・native_object_access_plan.md 5-2）。組み込みの `len` と同じ `len_of`（唯一の実装）を呼び、
/// 結果を生の int で返す。⚠ 以前の生成コードは `len` を `CB_GET_GLOBAL` で値として引き、`CB_CALL_FN` で呼んで
/// ハンドルを受け、`CB_TO_INT` で開けていた。
extern "C" fn ar_len(h: i64) -> i64 {
    let (has_err, v, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.clone_value(h), st.interp_ptr)
    });
    if has_err { return 0; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for len".to_string()));
        return 0;
    }
    match unsafe { &mut *interp_ptr }.len_of(v) {
        Ok(Value::Int(n)) => n,
        Ok(other) => {
            with_state(|s| s.borrow_mut().error = Some(format!("TypeError: len() returned a non-int value: {other:?}")));
            0
        }
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            0
        }
    }
}

/// `str(<int>)`（field 69・5-2）。組み込みの `str` と同じ `call_type_by_name_evaled("str", ..)` を、値を箱に入れずに呼ぶ。
extern "C" fn ar_str_of_int(n: i64) -> i64 {
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some("NativeError: interpreter not set for str".to_string()));
        return TL_NONE;
    }
    match unsafe { &mut *interp_ptr }.call_type_by_name_evaled("str", vec![Value::Int(n)]) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
            TL_NONE
        }
    }
}

extern "C" fn ar_flat_data_ptr(h: i64) -> i64 {
    if with_state(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().data.as_ptr() as i64,
        _ => 0,
    }
}

extern "C" fn ar_flat_len(h: i64) -> i64 {
    if with_state(|s| s.borrow().error.is_some()) { return 0; }
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
    let (has_err, interp_ptr) = with_state(|s| {
        let st = s.borrow();
        (st.error.is_some(), st.interp_ptr)
    });
    if has_err { return TL_NONE; }
    if interp_ptr.is_null() {
        with_state(|s| s.borrow_mut().error = Some(format!("NativeError: interpreter not set (calling '{name}')")));
        return TL_NONE;
    }
    let args: Vec<Value> = (0..n_args.max(0) as usize)
        .map(|i| clone_value_at(unsafe { *args_ptr.add(i) }))
        .collect();
    let interp = unsafe { &mut *interp_ptr };
    match interp.call_builtin_by_name_evaled(name, args) {
        Ok(v) => push_handle(v),
        Err(e) => {
            with_state(|s| s.borrow_mut().error = Some(e));
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
    make_str_ic: ar_make_str_ic,
    call_method_ic: ar_call_method_ic,
    assign_path_ic: ar_assign_path_ic,
    set_fresh: ar_set_fresh,
    copy_fresh: ar_copy_fresh,
    borrow_or_copy: ar_borrow_or_copy,
    inst_block: ar_inst_block,
    attr_block: ar_attr_block,
    expect_ret: ar_expect_ret,
    live_tasks: &crate::interpreter::async_mgr::LIVE_TASKS,
    copy_hooks: &crate::interpreter::value::COPY_HOOKS,
    new_instance: ar_new_instance,
    iter_next_at: ar_iter_next_at,
    num_field_ptr: ar_num_field_ptr,
    attr_blocks: ar_attr_blocks,
    iter_begin: ar_iter_begin,
};

/// field 86（copy_alloc_speed_plan.md 7-18）: 同じ親の `n` 個のフィールドが持つインスタンスの先頭の塊の番地を、親を 1 回だけ
/// たどって `out[i]` へ書く。1 件ごとの結果は field 79 を `reqs[i]` で呼んだのと同じ（読み出しの IC が当たり、slot が今持って
/// いる値が表のとおりのクラスのインスタンスのときだけ番地・ほかは空）。先に積まれた誤りがある・親がインスタンスでないなら
/// 全部空。アリーナには積まない。
/// ⚠ 外れた件をここで読み直さない（field 79 と同じ理由）。生成コードがその件の IC のセルで field 58 を呼んで埋める。
/// ⚠ 親も子も借りずに読む（`try_borrow_unguarded`・7-22）。可変に借りられているインスタンスはその件を空にする（今までの道へ）。
///   以前は `borrow()` で借りていた: 借用の印を書いて戻す 2 回の書き込みと、`instance_block` の呼び出し・`out.fill` の
///   `memset` が、1 回ごとの固定費だった（借りられていたら panic だった）
extern "C" fn ar_attr_blocks(parent_h: i64, n: i32, reqs: *const AttrBlockReq, out: *mut *mut u64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    if n <= 0 || reqs.is_null() || out.is_null() {
        return;
    }
    // SAFETY: 生成コードは `n` 件の表と `n` 個の `ptr` の置き場を渡す
    let reqs = unsafe { std::slice::from_raw_parts(reqs, n as usize) };
    let out = unsafe { std::slice::from_raw_parts_mut(out, n as usize) };
    let clear = |out: &mut [*mut u64]| {
        for o in out.iter_mut() {
            *o = std::ptr::null_mut();
        }
    };
    if parent_h < 3 {
        clear(out);
        return;
    }
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            clear(out);
            return;
        }
        let Some(Value::Instance(rc)) = st.arena.get(parent_h as usize) else {
            clear(out);
            return;
        };
        // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない（コールバックの中で利用者のコードを呼ばない）
        let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else {
            clear(out);
            return;
        };
        for (r, slot) in reqs.iter().zip(out.iter_mut()) {
            *slot = std::ptr::null_mut();
            if r.ic_cell.is_null() || r.class_cell.is_null() || r.desc.is_null() {
                continue;
            }
            // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（`read_ic_hit` と同じ）
            let packed = unsafe { &*(r.ic_cell as *const AtomicU64) }.load(Ordering::Relaxed);
            let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(r.name, r.name_len as i32)) else {
                continue;
            };
            if let Some(Some((Value::Instance(child), _))) = inst.boxed_fields.get(idx) {
                *slot = instance_block(child, r.desc, r.class_cell);
            }
        }
    })
}

/// field 85（copy_alloc_speed_plan.md 7-16）: `x.f op= v`（数値のフィールド）を生成コードが 1 回で行うための、フィールドの
/// 値の番地。返すのは、受け手が raw レイアウトでないクラスのインスタンスで、読み出しの IC が当たり（public のフィールド）、
/// スロットが初期化済みで可変の印が立ち（`let` のフィールド・凍ったインスタンスは印が倒れている）、`kind` どおりの
/// `Value::Int`（0）/ `Value::Float`（1）を持つときだけ。生成コードはそこへ `old op v` を直接書く: 値の型は変わらず（宣言の
/// 型の検査 `store_field` を通る値と同じ）、可変の印もそのまま。ほかは空を返し、生成コードは今までの道（同じ IC のセルで
/// 読む・`attr_assign_ic` で書く）へ回る。先に積まれた誤りがあれば空（今までの道が何もしない）。
/// ⚠ 番地は、次にコールバックを呼ぶまでの間だけ使える（スロットを差し替えるのはコールバックだけ）。
/// ⚠ 借りられない（誰かが借りたまま）なら空。
extern "C" fn ar_num_field_ptr(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64, kind: i32) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if cell.is_null() || obj_h < 3 || name_len <= 0 || name_ptr.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: `read_ic_hit` と同じ（生成コードが渡すのは DLL 内の `i64` の大域）
    let packed = unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed);
    if packed == 0 {
        return std::ptr::null_mut();
    }
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        let Some(Value::Instance(rc)) = st.arena.get(obj_h as usize) else { return std::ptr::null_mut() };
        let Ok(mut inst) = rc.try_borrow_mut() else { return std::ptr::null_mut() };
        if inst.has_raw_layout() {
            return std::ptr::null_mut();
        }
        let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len)) else {
            return std::ptr::null_mut();
        };
        let p = match (kind, inst.boxed_fields.get_mut(idx)) {
            (0, Some(Some((Value::Int(n), true)))) => (n as *mut i64).cast::<u64>(),
            (1, Some(Some((Value::Float(f), true)))) => (f as *mut f64).cast::<u64>(),
            _ => return std::ptr::null_mut(),
        };
        // 格納の入口の数え方は今までの道（`assign_number_ic_hit`）と同じにする（数値は写さない・番地を返すときだけ数える）
        if let Some(Some((v, _))) = inst.boxed_fields.get(idx) {
            crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::Store, v, true);
        }
        p
    })
}

/// field 80（7-5）: ガードが外れて名前で呼んだメソッドの戻り値を、宣言の型どおりに開ける。
///
/// ガード付きの呼び出し（受け手のクラスを型検査の注釈で決めた呼び出し）は、当たりの道で `_impl` が返す生の `int` / `float`
/// をそのまま使う。外れるのは注釈と違うクラスの値が来たとき（型検査の穴を通ったとき）だけで、そのとき呼んだ別のクラスの
/// メソッドが宣言と違う型を返したら、黙って変換せずに `TypeError` にする（以前は `int` は `to_int` で切り捨て、`float` は
/// 箱のまま続けていた・旧 known_problems.md #1）。
/// ⚠ 先に積まれたエラー・例外があれば何もしない（呼んだメソッドが投げた例外をこの誤りで上書きしない）。
/// ⚠ `int` に `bool` は通さない（Arrow の `bool` は `int` ではない）。`float` に `int` も通さない（VM はそのまま `int` で続ける）。
extern "C" fn ar_expect_ret(h: i64, kind: i32, name_ptr: *const u8, name_len: i32) -> i64 {
    let (has_err, got) = with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() || st.pending_raise.is_some() {
            return (true, None);
        }
        let got = match (kind, h) {
            (0, n) if n >= 3 && (n as usize) < INT_CACHE_BASE => Some(n - 3),
            (0, n) if n >= 3 => match st.arena.get(n as usize) {
                Some(Value::Int(v)) => Some(*v),
                Some(Value::UInt(v)) => Some(*v as i64),
                _ => None,
            },
            (1, n) if n >= INT_CACHE_BASE as i64 => match st.arena.get(n as usize) {
                Some(Value::Float(f)) => Some(f.to_bits() as i64),
                _ => None,
            },
            _ => None,
        };
        (false, got)
    });
    if has_err {
        return 0;
    }
    if let Some(v) = got {
        return v;
    }
    let want = if kind == 1 { "float" } else { "int" };
    let name = native_str(name_ptr, name_len, "method name").unwrap_or("?");
    set_argument_error(format!(
        "TypeError: method '{name}' is declared to return '{want}', but the call returned '{}'",
        handle_type_name(h)
    ));
    0
}

/// field 79（8-7）: 親の `name` のフィールドが持っているインスタンスの先頭の塊の番地。アリーナには積まない。
///
/// 返すのは、読み出しの IC が当たり（セルが空・クラスが違う・public でないなら当たらない）、その slot が**今持っている値**が
/// 表のとおりのクラスのインスタンスのときだけ。外れたときにここで読み直さない: 外れた道（`get_attr_val`）は一時値を返すことが
/// あり（ディスクリプタ・`import[rs]` のフィールド）、積まずに番地だけ返すと、持ち主のいない塊を指す。外れたら空を返し、
/// 生成コードが同じセルで field 58（`ar_get_attr_ic`）を呼ぶ（そちらがセルを埋め、値をアリーナに積む）。
extern "C" fn ar_attr_block(
    parent_h: i64,
    name_ptr: *const u8,
    name_len: i32,
    ic_cell: *mut u64,
    desc: *const crate::interpreter::value::DirectClassDesc,
    class_cell: *mut u64,
) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    if parent_h < 3 || ic_cell.is_null() || class_cell.is_null() || desc.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（`read_ic_hit` と同じ）
    let packed = unsafe { &*(ic_cell as *const AtomicU64) }.load(Ordering::Relaxed);
    with_state(|s| {
        let st = s.borrow();
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        let Some(Value::Instance(rc)) = st.arena.get(parent_h as usize) else { return std::ptr::null_mut() };
        // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない（可変に借りられていれば空・今までの道へ）
        let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else { return std::ptr::null_mut() };
        let Some((idx, _)) = inst.ic_slot(packed, true, || debug_field_name(name_ptr, name_len)) else {
            return std::ptr::null_mut();
        };
        // インスタンスを持てるのは raw レイアウトでないクラスの slot だけ（raw レイアウトのフィールドは数値）
        match inst.boxed_fields.get(idx) {
            Some(Some((Value::Instance(child), _))) => instance_block(child, desc, class_cell),
            _ => std::ptr::null_mut(),
        }
    })
}

// ── 番地から直接読み書きする道（field 78・copy_alloc_speed_plan.md 8-2・D-13） ──────────────

/// セルに覚える値: 確かめたクラスの番号に印を付けたもの（0 は「まだ」なので、番号 0 のクラスと区別する）。
fn direct_cell_value(class_id: u32) -> u64 {
    (1u64 << 32) | class_id as u64
}

/// 生成コードの表を読む: (クラスの名前, フィールドごとの (名前, `direct_field_info`))。UTF-8 でない名前・負の長さは `None`。
///
/// # Safety
/// `desc` は [`DirectClassDesc`] と、それに続く `n_fields` 個の [`DirectFieldDesc`]（生成コードが DLL の中に置く定数）を指すこと。
unsafe fn read_direct_desc<'a>(
    desc: *const crate::interpreter::value::DirectClassDesc,
) -> Option<(&'a str, Vec<(&'a str, u64)>)> {
    use crate::interpreter::value::{DirectClassDesc, DirectFieldDesc};
    let text = |p: *const u8, len: i64| -> Option<&'a str> {
        if p.is_null() || len <= 0 {
            return None;
        }
        // SAFETY: 関数の契約（生成コードが `len` バイトの定数を渡す）
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(p, len as usize) }).ok()
    };
    // SAFETY: 関数の契約
    let head: &DirectClassDesc = unsafe { &*desc };
    // raw レイアウトは 24 フィールドまで（`RawLayout::from_fields`）。それを超える表は読まない
    if !(1..=24).contains(&head.n_fields) {
        return None;
    }
    let name = text(head.name, head.name_len)?;
    // SAFETY: 表は先頭の直後に `n_fields` 個続く（並びは `repr(C)` で、どちらも 8 バイト境界）
    let fields = unsafe { std::slice::from_raw_parts(desc.add(1) as *const DirectFieldDesc, head.n_fields as usize) };
    let fields = fields.iter().map(|f| Some((text(f.name, f.name_len)?, f.info))).collect::<Option<Vec<_>>>()?;
    Some((name, fields))
}

/// field 78 の本体: インスタンスのクラスが生成コードの表のとおりなら、先頭の塊の番地。
#[inline(always)]
fn instance_block(
    rc: &std::rc::Rc<std::cell::RefCell<crate::interpreter::value::InstanceData>>,
    desc: *const crate::interpreter::value::DirectClassDesc,
    cell: *mut u64,
) -> *mut u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: 生成コードが渡すのは DLL 内の `i64` の大域（8 バイト境界）で、`AtomicU64` と同じ並び。
    let cell = unsafe { &*(cell as *const AtomicU64) };
    // SAFETY: 読むだけで、読み終わるまで誰も可変に借りない
    let Ok(inst) = (unsafe { rc.try_borrow_unguarded() }) else { return std::ptr::null_mut() };
    // ⚠ クラスは `inst.class` の番号で見る。塊の先頭の語の番号で見る形を試すと k1 が 0.94x になった（`class` は
    //   どのインスタンスでも同じで速い記憶にあるが、塊はインスタンスごとの行で、ここでその行を待つことになる・7-22）
    let want = direct_cell_value(inst.class.class_id);
    if cell.load(Ordering::Relaxed) != want && !fill_direct_cell(inst, desc, cell, want) {
        return std::ptr::null_mut();
    }
    // SAFETY: 塊（`Box<[u64]>`）はインスタンスが生きている間動かない（作るのは生成と深いコピーだけ）。
    // 生成コードは、この番地を使う間そのインスタンスを借りている者がいない所でだけ読み書きする（D-13 ④）。
    unsafe { (*rc.as_ptr()).raw.as_mut_ptr() }
}

/// [`instance_block`] のセルが外れたとき: 生成コードの表とクラスを突き合わせ、合えばセルを埋めて `true`。
#[cold]
#[inline(never)]
fn fill_direct_cell(
    inst: &crate::interpreter::value::InstanceData,
    desc: *const crate::interpreter::value::DirectClassDesc,
    cell: &std::sync::atomic::AtomicU64,
    want: u64,
) -> bool {
    // SAFETY: `desc` は生成コードが DLL の中に置いた表
    let Some((name, fields)) = (unsafe { read_direct_desc(desc) }) else { return false };
    if !inst.class.matches_direct_desc(name, &fields) {
        return false;
    }
    cell.store(want, std::sync::atomic::Ordering::Relaxed);
    true
}

extern "C" fn ar_inst_block(h: i64, desc: *const crate::interpreter::value::DirectClassDesc, cell: *mut u64) -> *mut u64 {
    if h < 3 || cell.is_null() || desc.is_null() {
        return std::ptr::null_mut();
    }
    with_state(|s| {
        let st = s.borrow();
        // 先に積まれたエラーがあれば直接の道に入れない（今までの道が何もせずに戻る）
        if st.error.is_some() {
            return std::ptr::null_mut();
        }
        match st.arena.get(h as usize) {
            Some(Value::Instance(rc)) => instance_block(rc, desc, cell),
            _ => std::ptr::null_mut(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// field 85（copy_alloc_speed_plan.md 7-16）: 数値のフィールドの値の番地。返すのは IC が当たり（public）、スロットが
    /// 初期化済みで可変の印が立ち、種類どおりの `Int` / `Float` を持つときだけ。番地へ書いた値はフィールドの値になる。
    /// `let` のフィールド・凍ったインスタンス・未初期化・種類の違い・private・空のセル・インスタンスでない・先に積まれた
    /// 誤りは空（生成コードは今までの道へ回る）。
    #[test]
    fn num_field_ptr_only_for_initialized_mutable_numbers() {
        use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData};
        let cls = Rc::new(ClassValue {
            field_index: [("hits", 0usize), ("mass", 1), ("id", 2), ("unset", 3)]
                .into_iter()
                .map(|(n, i)| (n.to_string(), i))
                .collect(),
            field_count: 4,
            field_mutability_vec: vec![true, true, false, true],
            ..ClassValue::synthetic("Counter", alloc_class_id())
        });
        let new_counter = || {
            let mut inst = InstanceData::new_empty(cls.clone(), 0);
            assert!(inst.store_field(0, Value::Int(7), true));
            assert!(inst.store_field(1, Value::Float(1.5), true));
            assert!(inst.store_field(2, Value::Int(9), false));
            Rc::new(RefCell::new(inst))
        };
        let cell = |idx: usize, access: u8| {
            let c = crate::ast::AttrCache::default();
            c.fill(cls.class_id, idx, access);
            c.0.get()
        };
        let field = |rc: &Rc<RefCell<InstanceData>>, idx: usize| rc.borrow().field_value(idx);
        let ptr = |h: i64, name: &str, c: &mut u64, kind: i32| ar_num_field_ptr(h, name.as_ptr(), name.len() as i32, c, kind);
        let rc = new_counter();
        let b = push_handle(Value::Instance(rc.clone()));
        let (mut hits, mut mass, mut id, mut unset) = (cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0));

        // `int` / `float`: 番地へ書いた値がフィールドの値になる
        let p = ptr(b, "hits", &mut hits, 0);
        assert!(!p.is_null());
        unsafe { *p.cast::<i64>() += 5 };
        assert!(matches!(field(&rc, 0), Some(Value::Int(12))));
        let p = ptr(b, "mass", &mut mass, 1);
        assert!(!p.is_null());
        unsafe { *p.cast::<f64>() += 1.0 };
        assert!(matches!(field(&rc, 1), Some(Value::Float(f)) if f == 2.5));
        // 種類の違い・`let` のフィールド・未初期化
        assert!(ptr(b, "hits", &mut hits, 1).is_null());
        assert!(ptr(b, "mass", &mut mass, 0).is_null());
        assert!(ptr(b, "id", &mut id, 0).is_null());
        assert!(ptr(b, "unset", &mut unset, 0).is_null());
        // private のフィールド（IC の access が public でない）・空のセル・インスタンスでない
        let mut private = cell(0, 1);
        assert!(ptr(b, "hits", &mut private, 0).is_null());
        let mut empty = 0u64;
        assert!(ptr(b, "hits", &mut empty, 0).is_null());
        assert!(ptr(TL_NONE, "hits", &mut hits, 0).is_null());
        // 凍ったインスタンス（スロットの可変の印が倒れる）
        let frozen = new_counter();
        crate::interpreter::Interpreter::freeze_instance(&frozen);
        let fh = push_handle(Value::Instance(frozen.clone()));
        assert!(ptr(fh, "hits", &mut hits, 0).is_null());
        // 先に積まれた誤りがあれば空（誤りはそのまま）
        set_native_error("ValueError: earlier".to_string());
        assert!(ptr(b, "hits", &mut hits, 0).is_null());
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
        assert!(matches!(field(&rc, 0), Some(Value::Int(12))));
    }

    /// field 84（copy_alloc_speed_plan.md 7-15）: 保存点まで巻き戻してから次の要素。反復の写しの要素は取り出して積み、
    /// 元のリストは変わらない。保存点 -1 は巻き戻さない（field 17 と同じ）。終わり（`TL_STOP_ITER`）でも巻き戻す。
    #[test]
    fn iter_next_at_rewinds_and_takes_elements_from_the_copy() {
        let items = std::rc::Rc::new(RefCell::new(vec![Value::str("a"), Value::str("b")]));
        let lh = push_handle(Value::List(items.clone()));
        let it = ar_iter_from(lh);
        let len = || with_state(|s| s.borrow().arena.len());
        let text = |h: i64| match clone_value_at(h) {
            Value::Str(s) => s.to_string(),
            other => panic!("not a str: {other:?}"),
        };
        let save = len();
        // 1 周目: 巻き戻さない（-1）。周の中で一時値を 1 つ積む
        let h1 = ar_iter_next_at(it, -1);
        assert_eq!(text(h1), "a");
        push_handle(Value::str("round temporary"));
        assert_eq!(len(), save + 2);
        // 2 周目: 1 周目の値（要素と一時値）を捨ててから次の要素を積む
        let h2 = ar_iter_next_at(it, save as i64);
        assert_eq!(h2, save as i64, "the next element takes the first freed slot");
        assert_eq!(text(h2), "b");
        assert_eq!(len(), save + 1);
        // 終わり: 巻き戻して `TL_STOP_ITER`
        assert_eq!(ar_iter_next_at(it, save as i64), TL_STOP_ITER);
        assert_eq!(len(), save);
        // 元のリストは要素を持ったまま（取り出したのは写しから）
        let kept: Vec<String> = items.borrow().iter().map(|v| match v {
            Value::Str(s) => s.to_string(),
            other => panic!("not a str: {other:?}"),
        }).collect();
        assert_eq!(kept, ["a", "b"]);
        // field 17 は今までどおり巻き戻さない
        let it2 = ar_iter_from(lh);
        let before = len();
        let h = ar_iter_next(it2);
        assert_eq!(text(h), "a");
        assert_eq!(len(), before + 1);
        ar_iter_drop(it2);
        ar_iter_drop(it);
        assert!(take_error().is_none());
    }

    /// field 87（copy_alloc_speed_plan.md 7-20）: 写す型は要素を全部アリーナへ積み、ハンドルの並び（番地と個数）を返す。
    /// 並びのハンドルは `push_value` と同じ（`None`・真偽値・小さい整数は決まったハンドル、ほかは順に新しい欄）で、要素は
    /// 入口のときのもの（後でリストを変えても並びの値は変わらない）。並びを field 84 で進めるのはエラー。写さない型（集合）は
    /// 個数 -1 で、field 84 が 1 つずつ取る。先に積まれた誤りがあれば `TL_NONE` と 0 個の並び。
    #[test]
    fn iter_begin_pushes_every_element_and_returns_the_handle_run() {
        let inner = std::rc::Rc::new(RefCell::new(vec![Value::Int(1)]));
        let items = std::rc::Rc::new(RefCell::new(vec![
            Value::None,
            Value::Bool(true),
            Value::Int(7),
            Value::Int(1_000_000),
            Value::str("s"),
            Value::List(inner.clone()),
        ]));
        let lh = push_handle(Value::List(items.clone()));
        let len = || with_state(|s| s.borrow().arena.len());
        let before = len() as i64;
        let mut out = [7i64, 7];
        let it = ar_iter_begin(lh, out.as_mut_ptr());
        assert!(it <= -2, "an iterator handle: {it}");
        assert_eq!(out[1], 6);
        // SAFETY: 並びは `iter_drop` まで動かない
        let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 6) }.to_vec();
        assert_eq!(&run[..3], &[TL_NONE, TL_TRUE, int_cache_handle(7)]);
        assert_eq!(&run[3..], &[before, before + 1, before + 2], "the other elements take new slots in order");
        assert_eq!(len() as i64, before + 3);
        assert!(matches!(clone_value_at(run[3]), Value::Int(1_000_000)));
        assert!(matches!(clone_value_at(run[4]), Value::Str(ref s) if &**s == "s"));
        assert!(matches!(clone_value_at(run[5]), Value::List(ref l) if std::rc::Rc::ptr_eq(l, &inner)), "the element itself");
        // 後でリストを変えても並びの値は変わらない（入口で要素の参照を足してある）
        items.borrow_mut().clear();
        assert!(matches!(clone_value_at(run[4]), Value::Str(ref s) if &**s == "s"));
        // 並びを field 84 で進めない
        assert_eq!(ar_iter_next_at(it, -1), TL_STOP_ITER);
        assert!(take_error().is_some_and(|e| e.contains("handle run")));
        ar_iter_drop(it);
        assert!(take_error().is_none());
        // 空のリスト: 0 個の並び
        let eh = push_handle(Value::List(std::rc::Rc::new(RefCell::new(Vec::new()))));
        let it = ar_iter_begin(eh, out.as_mut_ptr());
        assert!(it <= -2 && out[1] == 0, "{it} {out:?}");
        ar_iter_drop(it);
        // 写さない型（集合）は 1 つずつ（反復プロトコルはインタープリタを呼ぶ）
        let set = Value::Set(std::rc::Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2), Value::Int(3)])));
        let mut interp = crate::interpreter::Interpreter::new();
        let outermost = super::enter_native_call(&mut interp as *mut _);
        let sh = push_handle(set);
        let it = ar_iter_begin(sh, out.as_mut_ptr());
        assert!(it <= -2 && out[1] == -1, "{it} {out:?}");
        let mut sum = 0;
        loop {
            let h = ar_iter_next_at(it, -1);
            if h == TL_STOP_ITER {
                break;
            }
            match clone_value_at(h) {
                Value::Int(n) => sum += n,
                other => panic!("not an int: {other:?}"),
            }
        }
        assert_eq!(sum, 6);
        ar_iter_drop(it);
        assert!(take_error().is_none());
        // 先に積まれた誤り: 何も作らない
        set_native_error("ValueError: earlier".to_string());
        out = [7, 7];
        assert_eq!(ar_iter_begin(lh, out.as_mut_ptr()), TL_NONE);
        assert_eq!(out, [0, 0]);
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
        super::abort_native_call(outermost);
    }

    /// 並びの積み方（`push_run`・copy_alloc_speed_plan.md 7-21）: 空き欄へ直接書く種類（インスタンス・文字列・float・
    /// 整数）も、`Value::clone` で写す種類（リスト）も、決まったハンドル（`None`・真偽値・小さい整数）を挟んで順に
    /// 新しい欄へ入り、アリーナの長さは積んだ数だけ伸びる。要素は同じもの（`Rc` は参照の数が 1 つ増え、欄を捨てると戻る）。
    /// 写しから積む型（タプル）は写しの要素を動かして積む。
    #[test]
    fn iter_begin_writes_each_kind_into_its_own_slot() {
        use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData};
        let cls = Rc::new(ClassValue::synthetic("Item", alloc_class_id()));
        let inst = Rc::new(RefCell::new(InstanceData::new_empty(cls, 0)));
        let s: Rc<str> = Rc::from("text");
        let inner = Rc::new(RefCell::new(vec![Value::Int(1)]));
        let items = vec![
            Value::Instance(inst.clone()),
            Value::None,
            Value::Str(s.clone()),
            Value::Bool(false),
            Value::Float(2.5),
            Value::Int(255),
            Value::Int(256),
            Value::Int(-1),
            Value::List(inner.clone()),
            Value::Bool(true),
        ];
        let lh = push_handle(Value::List(Rc::new(RefCell::new(items.clone()))));
        let len = || with_state(|s| s.borrow().arena.len());
        let (inst_rc, str_rc, list_rc) = (Rc::strong_count(&inst), Rc::strong_count(&s), Rc::strong_count(&inner));
        let before = len() as i64;
        let mut out = [7i64, 7];
        let it = ar_iter_begin(lh, out.as_mut_ptr());
        assert!(it <= -2 && out[1] == 10, "{it} {out:?}");
        // SAFETY: 並びは `iter_drop` まで動かない
        let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 10) }.to_vec();
        let b = before;
        assert_eq!(
            run,
            [b, TL_NONE, b + 1, TL_FALSE, b + 2, int_cache_handle(255), b + 3, b + 4, b + 5, TL_TRUE],
            "fixed handles in between, the others in new slots in order"
        );
        assert_eq!(len() as i64, before + 6, "the arena grows by the pushed elements only");
        assert!(matches!(clone_value_at(run[0]), Value::Instance(ref r) if Rc::ptr_eq(r, &inst)));
        assert!(matches!(clone_value_at(run[2]), Value::Str(ref r) if Rc::ptr_eq(r, &s)));
        assert!(matches!(clone_value_at(run[4]), Value::Float(f) if f == 2.5));
        assert!(matches!(clone_value_at(run[6]), Value::Int(256)));
        assert!(matches!(clone_value_at(run[7]), Value::Int(-1)));
        assert!(matches!(clone_value_at(run[8]), Value::List(ref r) if Rc::ptr_eq(r, &inner)));
        // 参照の数は並びの分だけ 1 つ増える（`clone_value_at` の写しはもう捨ててある）
        assert_eq!(Rc::strong_count(&inst), inst_rc + 1);
        assert_eq!(Rc::strong_count(&s), str_rc + 1);
        assert_eq!(Rc::strong_count(&inner), list_rc + 1);
        ar_iter_drop(it);
        // 欄を捨てると戻る
        with_state(|st| st.borrow_mut().arena.truncate(before as usize));
        assert_eq!(Rc::strong_count(&inst), inst_rc);
        assert_eq!(Rc::strong_count(&s), str_rc);
        assert_eq!(Rc::strong_count(&inner), list_rc);
        // 写しから積む型（タプル）: 同じハンドルの並びになる
        let th = push_handle(Value::Tuple(Rc::new(crate::interpreter::TupleData::new(items))));
        let before = len() as i64;
        let it = ar_iter_begin(th, out.as_mut_ptr());
        assert!(it <= -2 && out[1] == 10, "{it} {out:?}");
        let run = unsafe { std::slice::from_raw_parts(out[0] as *const i64, 10) }.to_vec();
        let b = before;
        assert_eq!(run, [b, TL_NONE, b + 1, TL_FALSE, b + 2, int_cache_handle(255), b + 3, b + 4, b + 5, TL_TRUE]);
        assert_eq!(len() as i64, before + 6);
        assert!(matches!(clone_value_at(run[0]), Value::Instance(ref r) if Rc::ptr_eq(r, &inst)));
        assert!(matches!(clone_value_at(run[8]), Value::List(ref r) if Rc::ptr_eq(r, &inner)));
        ar_iter_drop(it);
        assert!(take_error().is_none());
    }

    /// ガードが外れたときの戻り値の開け方（field 80・7-5）: 宣言の型の値だけを開け、違う型は黙って変換せずに TypeError。
    /// 先に積まれたエラー・例外は上書きしない。
    #[test]
    fn expect_ret_unboxes_only_the_declared_type() {
        let name = "Body.kinetic";
        let call = |h: i64, kind: i32| ar_expect_ret(h, kind, name.as_ptr(), name.len() as i32);
        // int: 小さい整数（番号で表す）・大きい整数
        assert_eq!(call(push_handle(Value::Int(7)), 0), 7);
        assert_eq!(call(push_handle(Value::Int(1 << 40)), 0), 1 << 40);
        assert_eq!(call(push_handle(Value::Int(-3)), 0), -3);
        // float: ビット列で返る
        assert_eq!(f64::from_bits(call(push_handle(Value::Float(2.5)), 1) as u64), 2.5);
        assert!(take_error().is_none());
        // 宣言と違う型: float に int・int に float・bool・None・文字列
        for (h, kind, got) in [
            (push_handle(Value::Int(3)), 1, "int"),
            (push_handle(Value::Float(3.0)), 0, "float"),
            (TL_TRUE, 0, "bool"),
            (TL_NONE, 1, "NoneType"),
            (ar_make_str(b"x".as_ptr(), 1), 1, "str"),
        ] {
            assert_eq!(call(h, kind), 0);
            let want = if kind == 1 { "float" } else { "int" };
            assert_eq!(
                take_error(),
                Some(format!("TypeError: method 'Body.kinetic' is declared to return '{want}', but the call returned '{got}'"))
            );
        }
        // 先に積まれたエラーは上書きしない
        set_native_error("ValueError: raised inside the method".to_string());
        assert_eq!(call(TL_NONE, 1), 0);
        assert_eq!(take_error().as_deref(), Some("ValueError: raised inside the method"));
    }

    /// 受け手のクラスのガード（field 64）: セルが埋まっていればクラスの名前を読まない（7-9）。セルが空のときだけ名前を
    /// 検査し、正しくない UTF-8 ならエラー
    #[test]
    fn method_guard_reads_the_class_name_only_to_fill_the_cell() {
        use crate::interpreter::value::{alloc_class_id, ClassValue, InstanceData, INST_IMMUTABLE};
        use std::cell::RefCell;
        use std::rc::Rc;
        let cls = Rc::new(ClassValue::synthetic("Vec2", alloc_class_id()));
        let inst = |immutable: bool| {
            let mut d = InstanceData::new_empty(cls.clone(), 0);
            if immutable {
                d.set_flags(d.flags() | INST_IMMUTABLE);
            }
            push_handle(Value::Instance(Rc::new(RefCell::new(d))))
        };
        let bad = [0xFFu8, 0xFE];
        let mut cell = cls.class_id as u64;
        // セルが当たる: 名前（正しくない UTF-8）は読まず、エラーにもならない
        assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut cell, 0), 1);
        assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut cell, 1), 1);
        // `mut self` のメソッドに不変のインスタンス・別のクラスのセル・インスタンスでない値は外れ
        assert_eq!(ar_method_guard(inst(true), bad.as_ptr(), 2, &mut cell, 1), 0);
        let mut other = cell + 1;
        assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut other, 0), 0);
        assert_eq!(ar_method_guard(push_handle(Value::Int(1 << 40)), bad.as_ptr(), 2, &mut cell, 0), 0);
        assert!(take_error().is_none());
        // セルが空: 名前を検査する（正しくない UTF-8 はエラー・セルは空のまま）
        let mut empty = 0u64;
        assert_eq!(ar_method_guard(inst(false), bad.as_ptr(), 2, &mut empty, 0), 0);
        assert_eq!(empty, 0);
        assert!(take_error().is_some_and(|e| e.contains("class name from native code is not valid UTF-8")));
    }

    /// 合成した `__init__` の生成（field 83・7-11）: 宣言から静的に求めた格納の順（`static_class_layout`）が実行時の計画と
    /// 同じクラスにだけ、引数を格納する前のインスタンスを作って番地を返す。利用者が書いた `__init__`・違う順・違う表・
    /// クラスでない名前・先に積まれたエラーは 0（何も作らない・エラーを積まない）。
    #[test]
    fn new_instance_makes_only_instances_of_the_described_synthesized_init() {
        use crate::interpreter::value::{block, direct_field_info, static_class_layout, DirectClassDesc, DirectFieldDesc};
        let src = "class V2:\n    mut x: float\n    let n: int32\n    const K: int = 3\n\n\
                   class U2:\n    mut x: float\n    let n: int32\n\n    \
                   fn __init__(mut self, let x: float, let n: int32) -> None:\n        self.x = x\n        self.n = n\n\n\
                   fn not_a_class() -> int:\n    return 1\n";
        let stmts = crate::parser::Parser::new(crate::lexer::Lexer::new(src, "t.ar").tokenize(), None)
            .parse_program()
            .expect("parse");
        let layout = |name: &str| {
            stmts.iter().find_map(|s| match s {
                crate::ast::Stmt::ClassDef { name: n, bases, body, .. } if n == name => static_class_layout(bases, body),
                _ => None,
            })
        };
        // 静的な順: 合成した `__init__` は `const` を受けない。利用者が書いた同じ形の `__init__` は合成したものでない
        let v2 = layout("V2").expect("V2 has a raw layout");
        assert_eq!(v2.auto_init, Some(vec![0, 1]));
        assert_eq!(layout("U2").expect("U2 has a raw layout").auto_init, None);

        let mut interp = crate::interpreter::Interpreter::new();
        for s in &stmts {
            interp.exec(s).expect("exec");
        }
        let outermost = super::enter_native_call(&mut interp as *mut _);
        // 生成コードの表（V2 の並び。U2 も同じ並び）
        #[repr(C)]
        struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
        let table = |name: &'static str| {
            Table(
                DirectClassDesc { name: name.as_ptr(), name_len: name.len() as i64, n_fields: 2 },
                [
                    DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(v2.layout.fields[0], true) },
                    DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(v2.layout.fields[1], false) },
                ],
            )
        };
        let (tv, tu) = (table("V2"), table("U2"));
        // 1 つの箇所 = 1 組のセル・表・順（照合のセルは、その箇所の表と順で確かめたクラスを覚える）
        let call = |name: &str, t: &Table, order: &[i64], cells: &mut (u64, u64), out: &mut *mut u64| {
            let (cell, checked) = cells;
            ar_new_instance(cell, checked, name.as_ptr(), name.len() as i32, &t.0, order.as_ptr(), order.len() as i32, out)
        };

        // 表と順のとおり: インスタンスを積み、セルにクラスを覚え、先頭の塊の番地を返す。フィールドはまだ書いていない
        let (mut cells, mut out) = ((0u64, 0u64), std::ptr::null_mut());
        let h = call("V2", &tv, &[0, 1], &mut cells, &mut out);
        assert!(h >= 3 && !out.is_null());
        let Value::Instance(rc) = clone_value_at(h) else { panic!("not an instance") };
        let id1 = u64::from(rc.borrow().class.class_id) + 1;
        assert_eq!(cells, (id1, id1));
        assert_eq!(unsafe { (*rc.as_ptr()).raw.as_mut_ptr() }, out);
        let head = unsafe { *out };
        assert_eq!((head & block::CLASS_ID_MASK) as u32, rc.borrow().class.class_id);
        assert_eq!(head & (block::init_bit(0) | block::init_bit(1)), 0);
        assert!(rc.borrow().field_value(0).is_none());
        // 2 回目はセルから（名前は読まない: 正しくない UTF-8 でも作れる）
        let bad = [0xFFu8, 0xFE];
        let mut out2 = std::ptr::null_mut();
        let h2 = ar_new_instance(&mut cells.0, &mut cells.1, bad.as_ptr(), 2, &tv.0, [0i64, 1].as_ptr(), 2, &mut out2);
        assert!(h2 >= 3 && h2 != h && !out2.is_null() && out2 != out);

        // 外れ（それぞれ別の箇所）: 違う順・違う数・利用者が書いた `__init__`・表と違うクラス・クラスでない名前 → 0。
        // 何も作らず、照合のセルは空のまま、エラーも積まない
        let mut o = std::ptr::null_mut();
        for (name, t, order) in [
            ("V2", &tv, &[1i64, 0][..]),
            ("V2", &tv, &[0][..]),
            ("U2", &tu, &[0, 1][..]),
            ("V2", &tu, &[0, 1][..]),
            ("not_a_class", &tv, &[0, 1][..]),
        ] {
            let mut site = (0u64, 0u64);
            assert_eq!(call(name, t, order, &mut site, &mut o), 0, "{name} {order:?}");
            assert_eq!(site.1, 0);
        }
        assert!(o.is_null());
        assert!(take_error().is_none());
        // 先に積まれたエラーがあれば 0（エラーはそのまま残る）
        set_native_error("ValueError: earlier".to_string());
        assert_eq!(call("V2", &tv, &[0, 1], &mut cells, &mut o), 0);
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
        super::abort_native_call(outermost);
    }

    /// 同じ親の入れ子の受け手の番地をまとめて取るコールバック（field 86・copy_alloc_speed_plan.md 7-18）: 1 件ごとの結果は
    /// field 79（`ar_attr_block`）を同じ引数で呼んだのと同じ。表と違うクラスの子・インスタンスでない値・空の IC のセルの件だけ
    /// 空で、ほかの件は番地。親がインスタンスでない・先に積まれた誤りがあれば全部空（誤りはそのまま）。
    #[test]
    fn attr_blocks_match_attr_block_for_each_request() {
        use crate::interpreter::value::{
            alloc_class_id, direct_field_info, ClassValue, DirectClassDesc, DirectFieldDesc, InstanceData, RawLayout,
        };
        let make_raw = |name: &str, fields: &[(&str, &str)]| -> Rc<ClassValue> {
            let own: Vec<(String, String)> = fields.iter().map(|(n, a)| (n.to_string(), a.to_string())).collect();
            Rc::new(ClassValue {
                field_index: fields.iter().enumerate().map(|(i, (n, _))| (n.to_string(), i)).collect(),
                field_count: fields.len(),
                field_mutability_vec: vec![true; fields.len()],
                raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
                ..ClassValue::synthetic(name, alloc_class_id())
            })
        };
        let new_inst = |cls: &Rc<ClassValue>| {
            let mut inst = InstanceData::new_empty(cls.clone(), 0);
            let _ = inst.store_field(0, Value::Float(1.5), true);
            let _ = inst.store_field(1, Value::Int(7), true);
            Rc::new(RefCell::new(inst))
        };
        let vec2 = make_raw("Vec2", &[("x", "float"), ("n", "int32")]);
        // 同じ名前で並びの違うクラス（表と照合すると外れる）
        let swapped = make_raw("Vec2", &[("n", "int32"), ("x", "float")]);
        // 親（raw レイアウトでない）: v / w / s は子のインスタンス、k は数
        let parent = Rc::new(ClassValue {
            field_index: [("v", 0usize), ("w", 1), ("s", 2), ("k", 3)].into_iter().map(|(n, i)| (n.to_string(), i)).collect(),
            field_count: 4,
            field_mutability_vec: vec![true; 4],
            ..ClassValue::synthetic("H", alloc_class_id())
        });
        let mut pinst = InstanceData::new_empty(parent.clone(), 0);
        let v_child = new_inst(&vec2);
        assert!(pinst.store_field(0, Value::Instance(v_child.clone()), true));
        assert!(pinst.store_field(1, Value::Instance(new_inst(&vec2)), true));
        assert!(pinst.store_field(2, Value::Instance(new_inst(&swapped)), true));
        assert!(pinst.store_field(3, Value::Int(3), true));
        let prc = Rc::new(RefCell::new(pinst));
        let ph = push_handle(Value::Instance(prc.clone()));

        #[repr(C)]
        struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
        let layout = vec2.raw_layout.clone().unwrap();
        let table = Table(
            DirectClassDesc { name: "Vec2".as_ptr(), name_len: 4, n_fields: 2 },
            [
                DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[0], true) },
                DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[1], true) },
            ],
        );
        let desc = &table.0 as *const DirectClassDesc;
        let ic = |idx: usize| {
            let c = crate::ast::AttrCache::default();
            c.fill(parent.class_id, idx, 0);
            c.0.get()
        };
        // 件ごとの (名前, 読み出しの IC のセル, クラスのセル)。field 79 と同じ値を比べるため、セルは件ごとに別に持つ
        let names = ["v", "w", "s", "k"];
        let mut ics = [ic(0), ic(1), ic(2), ic(3), 0u64];
        let mut class_cells = [0u64; 5];
        let req = |name: &'static str, ic: *mut u64, cc: *mut u64| AttrBlockReq {
            name: name.as_ptr(),
            name_len: name.len() as i64,
            ic_cell: ic,
            desc,
            class_cell: cc,
        };
        let one = |name: &str, ic: &mut u64, cc: &mut u64| ar_attr_block(ph, name.as_ptr(), name.len() as i32, ic, desc, cc);
        let (mut ic79, mut cc79) = ([ic(0), ic(1), ic(2), ic(3)], [0u64; 4]);
        let want: Vec<*mut u64> = (0..4).map(|i| one(names[i], &mut ic79[i], &mut cc79[i])).collect();
        assert!(!want[0].is_null() && !want[1].is_null() && want[0] != want[1]);
        assert!(want[2].is_null() && want[3].is_null(), "a child of another layout / a number has no block");

        // 4 件 + IC のセルが空の件（v をもう 1 度・空のセル）
        let reqs = [
            req("v", &mut ics[0], &mut class_cells[0]),
            req("w", &mut ics[1], &mut class_cells[1]),
            req("s", &mut ics[2], &mut class_cells[2]),
            req("k", &mut ics[3], &mut class_cells[3]),
            req("v", &mut ics[4], &mut class_cells[4]),
        ];
        let mut out = [1usize as *mut u64; 5];
        ar_attr_blocks(ph, 5, reqs.as_ptr(), out.as_mut_ptr());
        assert_eq!(&out[..4], &want[..], "each request gets what field 79 returns");
        assert!(out[4].is_null(), "an empty IC cell gives null");
        // 親がインスタンスでない・先に積まれた誤り: 全部空（誤りはそのまま）
        for parent_h in [TL_NONE, push_handle(Value::Float(1.0))] {
            let mut out = [1usize as *mut u64; 2];
            ar_attr_blocks(parent_h, 2, reqs.as_ptr(), out.as_mut_ptr());
            assert!(out.iter().all(|p| p.is_null()));
        }
        // 借りられているインスタンス（7-22・借りずに読む）: 読むだけの借用は通る。可変の借用はその件だけ空（親なら全部空）で、
        // panic しない。field 79 も同じ。借用を返せば元どおり
        let blocks = || {
            let mut out = [1usize as *mut u64; 2];
            ar_attr_blocks(ph, 2, reqs.as_ptr(), out.as_mut_ptr());
            out
        };
        {
            let _shared = v_child.borrow();
            assert_eq!(&blocks()[..], &want[..2], "a shared borrow does not stop the read");
        }
        {
            let _held = v_child.borrow_mut();
            let out = blocks();
            assert!(out[0].is_null(), "a mutably borrowed child gives null");
            assert_eq!(out[1], want[1]);
            assert!(one("v", &mut ic79[0], &mut cc79[0]).is_null());
        }
        {
            let _held = prc.borrow_mut();
            assert!(blocks().iter().all(|p| p.is_null()), "a mutably borrowed parent gives nulls");
            assert!(one("w", &mut ic79[1], &mut cc79[1]).is_null());
        }
        assert_eq!(&blocks()[..], &want[..2]);
        assert_eq!(one("v", &mut ic79[0], &mut cc79[0]), want[0]);
        set_native_error("ValueError: earlier".to_string());
        let mut out = [1usize as *mut u64; 2];
        ar_attr_blocks(ph, 2, reqs.as_ptr(), out.as_mut_ptr());
        assert!(out.iter().all(|p| p.is_null()));
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
    }

    /// 番地を返すコールバック（field 78・8-2）: クラスが生成コードの表のとおりのときだけ番地を返し、セルに覚える。
    /// 表と 1 つでも違うクラス・インスタンスでない値・先に積まれたエラーがあるときは空。
    #[test]
    fn inst_block_returns_the_block_only_for_the_described_class() {
        use crate::interpreter::value::{
            alloc_class_id, block, direct_field_info, ClassValue, DirectClassDesc, DirectFieldDesc, InstanceData,
            RawLayout,
        };
        use std::cell::RefCell;
        use std::rc::Rc;
        let make_class = |name: &str, fields: &[(&str, &str, bool)]| -> Rc<ClassValue> {
            let own: Vec<(String, String)> = fields.iter().map(|(n, a, _)| (n.to_string(), a.to_string())).collect();
            Rc::new(ClassValue {
                field_index: fields.iter().enumerate().map(|(i, (n, _, _))| (n.to_string(), i)).collect(),
                field_count: fields.len(),
                field_mutability_vec: fields.iter().map(|(_, _, m)| *m).collect(),
                raw_layout: RawLayout::for_class(true, &own, own.len()).map(Rc::new),
                ..ClassValue::synthetic(name, alloc_class_id())
            })
        };
        let new_inst = |cls: &Rc<ClassValue>| {
            let mut inst = InstanceData::new_empty(cls.clone(), 0);
            // 並びの違うクラスでは型が合わず入らないものがある（番地が返らないことだけを見るので構わない）
            let _ = inst.store_field(0, Value::Float(1.5), true);
            let _ = inst.store_field(1, Value::Int(7), true);
            push_handle(Value::Instance(Rc::new(RefCell::new(inst))))
        };
        // 生成コードの表: 先頭 + フィールド 2 つ（`repr(C)` の並びのまま 1 つの定数にする）
        #[repr(C)]
        struct Table(DirectClassDesc, [DirectFieldDesc; 2]);
        let vec2 = make_class("Vec2", &[("x", "float", true), ("n", "int32", true)]);
        let layout = vec2.raw_layout.clone().unwrap();
        let table = Table(
            DirectClassDesc { name: "Vec2".as_ptr(), name_len: 4, n_fields: 2 },
            [
                DirectFieldDesc { name: "x".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[0], true) },
                DirectFieldDesc { name: "n".as_ptr(), name_len: 1, info: direct_field_info(layout.fields[1], true) },
            ],
        );
        let desc = &table.0 as *const DirectClassDesc;
        let mut cell = 0u64;

        // 表のとおりのクラス: 番地が返り、セルが埋まる。番地の先頭の語は class_id、フィールドは表の位置にある
        let h = new_inst(&vec2);
        let p = ar_inst_block(h, desc, &mut cell);
        assert!(!p.is_null());
        assert_eq!(cell, direct_cell_value(vec2.class_id));
        let head = unsafe { *p };
        assert_eq!((head & block::CLASS_ID_MASK) as u32, vec2.class_id);
        assert_ne!(head & block::init_bit(0), 0);
        let x_at = (table.1[0].info & 0xFFFF) as usize;
        assert_eq!(unsafe { (p as *const u8).add(x_at).cast::<f64>().read_unaligned() }, 1.5);
        // 2 回目は同じ番地（セルが当たる）
        assert_eq!(ar_inst_block(h, desc, &mut cell), p);

        // 同じ名前で並びの違うクラス（フィールドの順・幅・`let`・private）は空。セルは変わらない
        let mut private = make_class("Vec2", &[("x", "float", true), ("n", "int32", true)]);
        Rc::get_mut(&mut private).unwrap().field_access.insert("n".to_string(), crate::ast::Accessibility::Private);
        for other in [
            make_class("Vec2", &[("n", "int32", true), ("x", "float", true)]),
            make_class("Vec2", &[("x", "float", true), ("n", "int", true)]),
            make_class("Vec2", &[("x", "float", true), ("n", "int32", false)]),
            make_class("Vec3", &[("x", "float", true), ("n", "int32", true)]),
            make_class("Vec2", &[("x", "float", true), ("n", "int32", true), ("z", "float", true)]),
            private,
        ] {
            let h = new_inst(&other);
            assert!(ar_inst_block(h, desc, &mut cell).is_null(), "class {:?} must not match", other.field_index);
            assert_eq!(cell, direct_cell_value(vec2.class_id));
        }

        // インスタンスでない値・小さい番号・空の引数
        assert!(ar_inst_block(push_handle(Value::Float(1.0)), desc, &mut cell).is_null());
        assert!(ar_inst_block(TL_NONE, desc, &mut cell).is_null());
        assert!(ar_inst_block(h, std::ptr::null(), &mut cell).is_null());
        assert!(ar_inst_block(h, desc, std::ptr::null_mut()).is_null());
        // 可変に借りられているインスタンスは空で、panic しない（借りずに読む・7-22）。借用を返せば同じ番地
        let rc = match clone_value_at(h) {
            Value::Instance(rc) => rc,
            other => panic!("not an instance: {other:?}"),
        };
        {
            let _held = rc.borrow_mut();
            assert!(ar_inst_block(h, desc, &mut cell).is_null(), "a mutably borrowed instance gives null");
        }
        assert_eq!(ar_inst_block(h, desc, &mut cell), p);
        // 先に積まれたエラーがあれば空（エラーはそのまま残る）
        set_native_error("ValueError: earlier".to_string());
        assert!(ar_inst_block(h, desc, &mut cell).is_null());
        assert!(take_error().is_some_and(|e| e.contains("earlier")));
        assert!(!ar_inst_block(h, desc, &mut cell).is_null());

        // ── 数値の書き込みの命中経路（8-8）: 当たれば 1 回の借用で検査と格納。エラーの文面は今までと同じ ──
        {
            use crate::vm::op::TypeTag;
            let body = Rc::new(ClassValue {
                field_index: [("name", 0usize), ("mass", 1), ("hits", 2), ("id", 3)]
                    .into_iter()
                    .map(|(n, i)| (n.to_string(), i))
                    .collect(),
                field_count: 4,
                field_mutability_vec: vec![true, true, true, false],
                field_tags: vec![TypeTag::Str, TypeTag::Float, TypeTag::Int, TypeTag::Int],
                ..ClassValue::synthetic("Body", alloc_class_id())
            });
            let new_body = |flags: u32| {
                let mut inst = InstanceData::new_empty(body.clone(), 0);
                assert!(inst.store_field(0, Value::Str("b".into()), true));
                assert!(inst.store_field(1, Value::Float(1.0), true));
                assert!(inst.store_field(2, Value::Int(0), true));
                assert!(inst.store_field(3, Value::Int(9), false));
                inst.flags_or(flags);
                push_handle(Value::Instance(Rc::new(RefCell::new(inst))))
            };
            let field = |h: i64, idx: usize| match clone_value_at(h) {
                Value::Instance(rc) => rc.borrow().field_value(idx),
                other => panic!("{other:?}"),
            };
            let cell_for = |idx: usize| {
                let c = crate::ast::AttrCache::default();
                c.fill(body.class_id, idx, crate::ast::AttrCache::PUBLIC);
                c.0.get()
            };
            let (mut mass, mut hits, mut id) = (cell_for(1), cell_for(2), cell_for(3));
            let b = new_body(0);
            // 当たり: float・int・`int` を `float` のフィールドへ（昇格）
            ar_assign_float_field_ic(b, "mass".as_ptr(), 4, 2.5, &mut mass);
            ar_assign_int_field_ic(b, "hits".as_ptr(), 4, 7, &mut hits);
            assert!(take_error().is_none());
            assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 2.5));
            assert!(matches!(field(b, 2), Some(Value::Int(7))));
            ar_assign_int_field_ic(b, "mass".as_ptr(), 4, 3, &mut mass);
            assert!(take_error().is_none());
            assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
            // 型の合わない値・`let` のフィールド・不変のインスタンス: 共有の検査（`store_field_checked`）と同じ文面で、値は変わらない
            ar_assign_float_field_ic(b, "hits".as_ptr(), 4, 1.5, &mut hits);
            assert_eq!(take_error().as_deref(), Some("TypeError: value does not match declared type of field 'hits'"));
            assert!(matches!(field(b, 2), Some(Value::Int(7))));
            ar_assign_int_field_ic(b, "id".as_ptr(), 2, 1, &mut id);
            assert_eq!(take_error().as_deref(), Some("TypeError: cannot assign to immutable field 'id'"));
            assert!(matches!(field(b, 3), Some(Value::Int(9))));
            let frozen = new_body(crate::interpreter::value::INST_IMMUTABLE);
            ar_assign_float_field_ic(frozen, "mass".as_ptr(), 4, 9.0, &mut mass);
            // 同じ形のもう 1 つのインスタンスに、共有の検査を直接かけた結果と比べる（エラーも、格納された値も）
            let twin = new_body(crate::interpreter::value::INST_IMMUTABLE);
            let direct = {
                let Value::Instance(rc) = clone_value_at(twin) else { panic!() };
                let mut inst = rc.borrow_mut();
                crate::interpreter::Interpreter::store_field_checked(&mut inst, 1, "mass", Value::Float(9.0), true)
            };
            assert_eq!(take_error(), direct.err(), "the same error as the shared check");
            assert!(matches!((field(frozen, 1), field(twin, 1)), (Some(Value::Float(a)), Some(Value::Float(b))) if a == b));
            // 先に積まれたエラーがあれば書かない（エラーはそのまま）
            set_native_error("ValueError: earlier".to_string());
            ar_assign_float_field_ic(b, "mass".as_ptr(), 4, 8.0, &mut mass);
            assert!(take_error().is_some_and(|e| e.contains("earlier")));
            assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
            // 当たらない（別のクラスのセル・空のセル・インスタンスでない）ときは命中経路は何もしない
            let mut other = cell_for(1) ^ (1u64 << 40);
            assert!(!assign_number_ic_hit(b, "mass".as_ptr(), 4, Value::Float(8.0), &mut other));
            let mut empty = 0u64;
            assert!(!assign_number_ic_hit(b, "mass".as_ptr(), 4, Value::Float(8.0), &mut empty));
            assert!(!assign_number_ic_hit(TL_NONE, "mass".as_ptr(), 4, Value::Float(8.0), &mut mass));
            assert!(matches!(field(b, 1), Some(Value::Float(f)) if f == 3.0));
        }

        // ── 入れ子の受け手（field 79・8-7）: 親のフィールドが持っているインスタンスの番地。積まない ──
        let holder = Rc::new(ClassValue {
            field_index: [("p".to_string(), 0usize), ("tag".to_string(), 1)].into_iter().collect(),
            field_count: 2,
            field_mutability_vec: vec![true, true],
            ..ClassValue::synthetic("Holder", alloc_class_id())
        });
        let child = match clone_value_at(new_inst(&vec2)) {
            Value::Instance(rc) => rc,
            other => panic!("{other:?}"),
        };
        let mut hinst = InstanceData::new_empty(holder.clone(), 0);
        assert!(hinst.store_field(0, Value::Instance(child.clone()), true));
        assert!(hinst.store_field(1, Value::Int(3), true));
        let ph = push_handle(Value::Instance(Rc::new(RefCell::new(hinst))));
        let mut ic = 0u64;
        let mut ccell = 0u64;
        // 読み出しの IC のセルが空のうちは空（生成コードが field 58 で読んでセルを埋める）
        assert!(ar_attr_block(ph, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
        let cache = crate::ast::AttrCache::default();
        cache.fill(holder.class_id, 0, crate::ast::AttrCache::PUBLIC);
        ic = cache.0.get();
        let arena_len = with_state(|s| s.borrow().arena.len());
        let cp = ar_attr_block(ph, "p".as_ptr(), 1, &mut ic, desc, &mut ccell);
        assert_eq!(cp, unsafe { (*child.as_ptr()).raw.as_mut_ptr() }, "the block of the instance the field holds");
        assert_eq!(with_state(|s| s.borrow().arena.len()), arena_len, "nothing is pushed");
        // slot が持っている値がインスタンスでない・親が別のクラス（IC が外れる）・親がインスタンスでない
        cache.fill(holder.class_id, 1, crate::ast::AttrCache::PUBLIC);
        let mut ic_tag = cache.0.get();
        assert!(ar_attr_block(ph, "tag".as_ptr(), 3, &mut ic_tag, desc, &mut ccell).is_null());
        assert!(ar_attr_block(h, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
        assert!(ar_attr_block(TL_NONE, "p".as_ptr(), 1, &mut ic, desc, &mut ccell).is_null());
    }

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

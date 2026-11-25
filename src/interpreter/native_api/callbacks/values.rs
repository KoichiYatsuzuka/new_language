// native_api/callbacks/values.rs — 値を作る・真偽・演算・長さなど、値そのものを扱うコールバック。

use super::*;

pub(super) extern "C" fn ar_make_int(n: i64) -> i64 {
    push_handle(Value::Int(n))
}

pub(super) extern "C" fn ar_make_float(f: f64) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        st.push_value(Value::Float(f))
    })
}

pub(super) extern "C" fn ar_make_bool(b: i32) -> i64 {
    if b != 0 { TL_TRUE } else { TL_FALSE }
}

pub(super) extern "C" fn ar_make_str(ptr: *const u8, len: i32) -> i64 {
    let Some(text) = native_str(ptr, len, "string") else { return TL_NONE };
    with_state(|s| s.borrow_mut().push_value(Value::str(text)))
}

/// 文字列のリテラルを呼び出し箇所ごとに 1 回だけ作る（field 72・11-2・D-12）。
///
/// `cell`（DLL の中の箇所ごとの `u64`・プロセスに 1 つ）には、初めて評価したときにプロセスで一意の番号を入れる。
/// 文字列はスレッドの状態の表 `str_consts` のその番号の位置に置き、以後は参照数の加算で積む（VM の定数表と同じ）。
/// 別のスレッド（非同期タスクのワーカー）は同じ番号で自分の表に作る（`Rc<str>` はスレッドをまたげない）。
/// ⚠ 中身の検査（UTF-8）はスレッドごとに初めて作るときだけ（中身は箇所ごとに変わらない）。
pub(super) extern "C" fn ar_make_str_ic(cell: *mut u64, ptr: *const u8, len: i32) -> i64 {
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
pub(super) extern "C" fn ar_make_list(items_ptr: *const i64, n: i32) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let items: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let list = Rc::new(RefCell::new(items));
        st.push_value(Value::List(list))
    })
}

pub(super) extern "C" fn ar_make_tuple(items_ptr: *const i64, n: i32) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let elements: Vec<Value> = (0..n as usize)
            .map(|i| st.clone_value(unsafe { *items_ptr.add(i) }))
            .collect();
        let tuple = Rc::new(TupleData::new(elements));
        st.push_value(Value::Tuple(tuple))
    })
}

pub(super) extern "C" fn ar_make_dict(keys_ptr: *const i64, vals_ptr: *const i64, n: i32) -> i64 {
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

pub(super) extern "C" fn ar_make_none() -> i64 {
    TL_NONE
}

pub(super) extern "C" fn ar_is_truthy(h: i64) -> i32 {
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

pub(super) extern "C" fn ar_binop(op: i32, a: i64, b: i64) -> i64 {
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

pub(super) extern "C" fn ar_unop(op: i32, a: i64) -> i64 {
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

pub(super) extern "C" fn ar_is_type(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
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

/// `char*` / `wchar_t*` の戻り値（4-3・D-9）。`len` バイトを `bytes` へコピーする（借用なので解放しない・D-7）。
pub(super) extern "C" fn ar_make_bytes(ptr: *const u8, len: i64) -> i64 {
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
pub(super) extern "C" fn ar_make_ptr(address: u64, elem: *const u8, elem_len: i32) -> i64 {
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

// ── Feature-extension callbacks ──────────────────────────────────────────────

pub(super) extern "C" fn ar_list_append(list_h: i64, item_h: i64) -> i64 {
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

/// `len(x)`（field 68・native_object_access_plan.md 5-2）。組み込みの `len` と同じ `len_of`（唯一の実装）を呼び、
/// 結果を生の int で返す。⚠ 以前の生成コードは `len` を `CB_GET_GLOBAL` で値として引き、`CB_CALL_FN` で呼んで
/// ハンドルを受け、`CB_TO_INT` で開けていた。
pub(super) extern "C" fn ar_len(h: i64) -> i64 {
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
pub(super) extern "C" fn ar_str_of_int(n: i64) -> i64 {
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

pub(super) extern "C" fn ar_flat_data_ptr(h: i64) -> i64 {
    if with_state(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().data.as_ptr() as i64,
        _ => 0,
    }
}

pub(super) extern "C" fn ar_flat_len(h: i64) -> i64 {
    if with_state(|s| s.borrow().error.is_some()) { return 0; }
    match clone_value_at(h) {
        Value::FrozenList { state, .. } => state.borrow().len as i64,
        _ => 0,
    }
}

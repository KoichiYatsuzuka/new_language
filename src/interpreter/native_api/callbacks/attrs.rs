// native_api/callbacks/attrs.rs — 属性・添字・グローバルの読み書きのコールバック。

use super::*;

pub(super) extern "C" fn ar_get_attr(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
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
pub(super) extern "C" fn ar_set_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
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
pub(super) extern "C" fn ar_assign_attr(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64) {
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
pub(super) extern "C" fn ar_assign_attr_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, val_h: i64, cell: *mut u64) {
    let val = clone_value_at(val_h);
    assign_attr_ic_value(obj_h, name_ptr, name_len, val, cell);
}

/// `obj.attr = <int>`（IC 付き・field 65・4-2）。値はハンドルにせず `Value::Int` で渡す（中身は field 61 と同じ）。
pub(super) extern "C" fn ar_assign_int_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, v: i64, cell: *mut u64) {
    if assign_number_ic_hit(obj_h, name_ptr, name_len, Value::Int(v), cell) {
        return;
    }
    assign_attr_ic_value(obj_h, name_ptr, name_len, Value::Int(v), cell);
}

/// `obj.attr = <float>`（IC 付き・field 66・4-2）。
pub(super) extern "C" fn ar_assign_float_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, v: f64, cell: *mut u64) {
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
pub(super) fn assign_number_ic_hit(obj_h: i64, name_ptr: *const u8, name_len: i32, val: Value, cell: *mut u64) -> bool {
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
pub(super) extern "C" fn ar_assign_path_ic(root_h: i64, steps: *const i64, n_steps: i32, kind: i32, value: i64) {
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
pub(super) extern "C" fn ar_get_trait_attr(
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
pub(super) extern "C" fn ar_assign_trait_attr(
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

pub(super) extern "C" fn ar_subscript(obj_h: i64, key_h: i64) -> i64 {
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

pub(super) extern "C" fn ar_get_global(name_ptr: *const u8, name_len: i32) -> i64 {
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

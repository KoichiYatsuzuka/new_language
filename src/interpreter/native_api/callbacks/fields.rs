// native_api/callbacks/fields.rs — フィールドの読み出しのコールバック（型つき・インラインキャッシュつき）。

use super::*;

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

pub(super) extern "C" fn ar_get_float_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> f64 {
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

pub(super) extern "C" fn ar_get_int_field(obj_h: i64, name_ptr: *const u8, name_len: i32) -> i64 {
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

pub(super) extern "C" fn ar_get_attr_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
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
pub(super) fn debug_field_name<'a>(ptr: *const u8, len: i32) -> &'a str {
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
    take: impl FnOnce(&mut crate::interpreter::native_api::NativeCallState, Value) -> Option<T>,
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
pub(super) fn site_name<'a>(name_ptr: *const u8, name_len: i32, cell: *mut u64, what: &str) -> Option<&'a str> {
    use std::sync::atomic::{AtomicU64, Ordering};
    // SAFETY: `read_attr_ic` と同じ
    let filled = !cell.is_null() && unsafe { &*(cell as *const AtomicU64) }.load(Ordering::Relaxed) != 0;
    if filled && name_len > 0 && !name_ptr.is_null() {
        // SAFETY: 長さとポインタは上で確かめた。中身はセルを埋める前に `native_str` が UTF-8 として検査した
        return Some(unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize)) });
    }
    native_str(name_ptr, name_len, what)
}

pub(super) extern "C" fn ar_get_float_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> f64 {
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

pub(super) extern "C" fn ar_get_int_field_ic(obj_h: i64, name_ptr: *const u8, name_len: i32, cell: *mut u64) -> i64 {
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

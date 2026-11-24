// native_api/callbacks/exceptions.rs — 例外とセルのコールバック。

use super::*;

pub(super) extern "C" fn ar_raise_exc(type_h: i64, msg_h: i64) -> i64 {
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

pub(super) extern "C" fn ar_make_cell(init_h: i64) -> i64 {
    with_state(|s| {
        let mut st = s.borrow_mut();
        let val = st.clone_value(init_h);
        let cell = Rc::new(RefCell::new(vec![val]));
        st.push_value(Value::List(cell))
    })
}

pub(super) extern "C" fn ar_get_cell(cell_h: i64) -> i64 {
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

pub(super) extern "C" fn ar_set_cell(cell_h: i64, val_h: i64) {
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

/// field 88（known_bugs.md #28）: このスレッドに誤りか `raise` した例外が積まれていれば 1。生成コードが、例外を出しうる
/// モジュールの関数を直接呼んだ後に呼び、1 ならその関数からすぐ戻る（積まれたものは入口〔`eval/native.rs`〕が取り出す）。
pub(super) extern "C" fn ar_exc_pending() -> i32 {
    with_state(|s| {
        let st = s.borrow();
        i32::from(st.error.is_some() || st.pending_raise.is_some())
    })
}

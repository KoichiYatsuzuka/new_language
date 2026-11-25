// native_api/callbacks/arena.rs — ハンドルのアリーナの保存・詰め直しと、値の写し・束縛のコールバック。

use super::*;

// ── Arena save / compact helpers ────────────────────────────────────────────
//
// Called around every intra-module function call in generated code to prevent
// arena blowup.  Without these, each call leaves ~N×ops intermediate handles
// in the arena for the entire duration of the outermost native call.

pub(super) extern "C" fn ar_arena_save() -> u64 {
    with_state(|s| s.borrow().arena.len() as u64)
}

pub(super) extern "C" fn ar_arena_compact(h: i64, saved: u64) -> i64 {
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
pub(super) extern "C" fn ar_compact_many(handles_in: *const i64, n: i32, save: u64, handles_out: *mut i64) {
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
pub(super) extern "C" fn ar_copy_arg(h: i64) -> i64 {
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
pub(super) extern "C" fn ar_bind_value(mode: i32, h: i64) -> i64 {
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
pub(super) extern "C" fn ar_let_from_name(name_ptr: *const u8, name_len: i32, h: i64) -> i64 {
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

pub(super) extern "C" fn ar_deep_copy(h: i64) -> i64 {
    let val = clone_value_at(h);
    crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetSelf, &val, false);
    let copied = crate::interpreter::Interpreter::deep_copy_value(val);
    push_handle(copied)
}

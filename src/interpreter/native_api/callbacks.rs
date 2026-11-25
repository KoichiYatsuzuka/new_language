// native_api/callbacks.rs — ネイティブ DLL へ渡す C ABI コールバック(`extern "C" fn ar_*`)群と
// それらを束ねる `CALLBACKS` テーブル。全ての重い操作はインタープリタ経由でルーティングする。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{DictData, TupleData, Value},
};
use super::*;

mod values;
mod calls;
mod attrs;
mod iter;
mod arena;
mod convert;
mod exceptions;
mod fields;
mod construct;
mod blocks;

use self::values::*;
use self::calls::*;
use self::attrs::*;
use self::iter::*;
use self::arena::*;
use self::convert::*;
use self::exceptions::*;
use self::fields::*;
use self::construct::*;
use self::blocks::*;

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
    exc_pending: ar_exc_pending,
};

#[cfg(test)]
mod tests;

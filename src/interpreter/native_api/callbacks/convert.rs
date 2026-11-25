// native_api/callbacks/convert.rs — ハンドルと数値・bytes・ポインタ・C の構造体の変換と、C++ ブリッジの補助。

use super::*;

/// ネイティブの `int` を取り出す（C / Rust の整数の引数・ネイティブコンパイルの生成コードの開封）。
///
/// 数値どうしは C の暗黙の変換と同じ（`float` は切り捨て・`bool` は 0 / 1・幅への切り詰めは中継コード）。
/// ⚠ **数値でない値（`None`・`str`・`list`・`Ptr` …）はエラー**を設定して 0 を返す（7-12）。以前は黙って 0 にしていて、
///   素の `function` 型の変数を通して `fp.fp_is_even("abc")` が `True` を返していた（実測）。中継コードは
///   `pending_error` を見て C / Rust の関数を呼ばずに戻る。
pub(super) extern "C" fn ar_to_int(h: i64) -> i64 {
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
pub(super) extern "C" fn ar_to_float(h: i64) -> f64 {
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
pub(super) extern "C" fn ar_to_bool(h: i64) -> i32 {
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
pub(super) extern "C" fn ar_pending_error() -> i32 {
    with_state(|s| s.borrow().error.is_some() as i32)
}

/// `const char*` の引数（4-3・D-13）。`bytes` は NUL を付けたコピー（最外のネイティブ呼び出しが終わるまで有効）、
/// `None` は NULL（D-8）。`str` は文字コードを書かせるためにエラー（黙って UTF-8 にしない）。NUL を含む `bytes` は
/// C の側で途中で切れるのでエラー。誤りは `pending_error` で拾われ、C の関数は呼ばれない（7-12）。
pub(super) extern "C" fn ar_bytes_arg(h: i64) -> *const u8 {
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
pub(super) extern "C" fn ar_ptr_arg(h: i64, elem: *const u8, elem_len: i32) -> u64 {
    let Some(elem) = native_str(elem, elem_len, "pointer element type") else { return 0 };
    // 規則は型つき経路と共有（`value::ptr_arg_address`）
    crate::interpreter::value::ptr_arg_address(&clone_value_at(h), elem).unwrap_or_else(|e| {
        set_argument_error(e);
        0
    })
}

/// `ArPtrProtocol` の引数（ハンドル経路・4-4）。`Ptr` か `None` から構造体を組み立てて `out` へ書く（規則は型つき経路と共有の
/// `value::protocol_arg`）。要素の型名の文字列は最外のネイティブ呼び出しが終わるまで生かす。
pub(super) extern "C" fn ar_protocol_arg(h: i64, out: *mut u8, writable: i32) {
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
pub(super) extern "C" fn ar_protocol_done(h: i64, address_after: u64) {
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
pub(super) extern "C" fn ar_bytes_buf_arg(h: i64, len: *mut i64) -> *mut u8 {
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
pub(super) extern "C" fn ar_protocol_ret(raw: *const u8, structs: *const u8, structs_len: i32) -> i64 {
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
pub(super) extern "C" fn ar_make_raw_instance(cls_h: i64, ptr: *const u8, len: i64) -> i64 {
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
pub(super) extern "C" fn ar_make_instance(cls_h: i64, handles: *const i64, n: i32) -> i64 {
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
pub(super) fn handle_type_name(h: i64) -> String {
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
pub(super) fn set_argument_error(msg: String) {
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
pub(super) extern "C" fn ar_to_cstr(h: i64) -> *const u8 {
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

pub(super) extern "C" fn ar_write_handle(target_h: i64, new_val_h: i64) {
    if target_h < 3 { return; }
    with_state(|s| {
        let mut st = s.borrow_mut();
        let new_val = st.clone_value(new_val_h);
        if let Some(slot) = st.arena.get_mut(target_h as usize) {
            *slot = new_val;
        }
    });
}

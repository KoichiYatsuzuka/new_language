use super::*;

fn layout() -> Arc<RawLayout> {
    Arc::new(RawLayout::from_fields(&[("x".into(), "int32".into())]).unwrap())
}

/// 構造体ポインタにインスタンス以外が来たら、ハンドル経路へ落とさずエラー（ffi_value_exchange_plan.md 1-2）。
/// 値渡しの構造体は従来どおりハンドル経路へ（フィールドから組み立てられる・1-3）。
#[test]
fn struct_pointer_needs_an_instance() {
    let mut ta = TypedArgs::new();
    let ptr = AbiTy::Ptr { mutable: false, by_value: false, layout: layout(), elem: Arc::from("P") };
    let err = ta.marshal(&[Value::Int(5)], &[ptr], &[]).err().expect("must be an error");
    assert!(err.contains("instance of the C struct") && err.contains("or a Ptr") && err.contains("'int'"), "{err}");

    let mut ta = TypedArgs::new();
    let by_value = AbiTy::Ptr { mutable: false, by_value: true, layout: layout(), elem: Arc::from("P") };
    assert!(matches!(ta.marshal(&[Value::Int(5)], &[by_value], &[]), Ok(Marshalled::TypeMismatch)));
}

fn borrowed(address: u64, elem: &str) -> Value {
    let h = crate::interpreter::PtrHandle::new(address, elem, 0, None, crate::interpreter::PtrOwner::Borrowed, None)
        .unwrap();
    Value::Ptr(Arc::new(h))
}

/// 型つき経路の `bool`・`bytes`・`Ptr`（4-4）。規則はハンドル経路と共有（`value/ptr.rs`）。
#[test]
fn typed_bool_bytes_and_pointers() {
    let mut ta = TypedArgs::new();
    let vals = [Value::Bool(true), Value::bytes(b"ab".to_vec()), Value::None, borrowed(0x40, "Db"), Value::None];
    let tys = [
        AbiTy::Bool,
        AbiTy::Bytes,
        AbiTy::Bytes,
        AbiTy::PtrVal { elem: Arc::from("Db") },
        AbiTy::PtrVal { elem: Arc::from("void") },
    ];
    assert!(matches!(ta.marshal(&vals, &tys, &[]), Ok(Marshalled::Ready)));
    assert_eq!(ta.slots[0], 1);
    // SAFETY: スロットは `ta.cleanups` が生かしている NUL 終端のコピーを指す
    let s = unsafe { std::ffi::CStr::from_ptr(ta.slots[1] as usize as *const std::ffi::c_char) };
    assert_eq!(s.to_bytes(), b"ab");
    assert_eq!(ta.slots[2], 0, "None is NULL");
    assert_eq!(ta.slots[3], 0x40);
    assert_eq!(ta.slots[4], 0);
    // 合わない値はハンドル経路へ落とさずエラー
    let mut ta = TypedArgs::new();
    let e = ta.marshal(&[Value::str("x")], &[AbiTy::Bytes], &[]).err().unwrap();
    assert!(e.contains("needs bytes, not 'str'"), "{e}");
    let e = ta.marshal(&[borrowed(0x40, "Db")], &[AbiTy::PtrVal { elem: Arc::from("File") }], &[]).err().unwrap();
    assert!(e.contains("Ptr[File]"), "{e}");
    // 構造体ポインタへ `Ptr[構造体]`（値渡しは None を受けない）
    let ptr = AbiTy::Ptr { mutable: false, by_value: false, layout: layout(), elem: Arc::from("P") };
    let mut ta = TypedArgs::new();
    assert!(matches!(ta.marshal(&[borrowed(0x80, "P")], &[ptr], &[]), Ok(Marshalled::Ready)));
    assert_eq!(ta.slots[0], 0x80);
    let by_value = AbiTy::Ptr { mutable: false, by_value: true, layout: layout(), elem: Arc::from("P") };
    let e = TypedArgs::new().marshal(&[Value::None], &[by_value], &[]).err().unwrap();
    assert!(e.contains("by value"), "{e}");
}

/// `ArPtrProtocol` の引数（4-4）: `Ptr` から組み立て、書き込み用ポインタで C が `address` を 0 にしたら使えなくする。
/// 値渡し・`const` では解放関数を渡さない。
#[test]
fn typed_protocol_arguments() {
    let p = borrowed(0x100, "int32");
    let mut ta = TypedArgs::new();
    assert!(matches!(
        ta.marshal(&[p.clone(), p.clone()], &[AbiTy::Protocol { writable: false }, AbiTy::Protocol { writable: true }], &[]),
        Ok(Marshalled::Ready)
    ));
    // SAFETY: スロットは `ta.cleanups` が生かしている構造体を指す
    let ro = unsafe { &*(ta.slots[0] as usize as *const super::super::PtrProtocolRaw) };
    assert_eq!(ro.address, 0x100);
    assert_eq!(unsafe { std::ffi::CStr::from_ptr(ro.elem_type) }.to_bytes(), b"int32");
    assert_eq!(ro.length, -1, "a raw pointer has no length");
    assert!(ro.release.is_none());
    // C の側で解放して address を 0 に書き戻した
    let rw = unsafe { &mut *(ta.slots[1] as usize as *mut super::super::PtrProtocolRaw) };
    rw.address = 0;
    for c in std::mem::take(&mut ta.cleanups) {
        finish_ptr_arg_cleanup(c);
    }
    let Value::Ptr(h) = &p else { unreachable!() };
    assert!(!h.is_live(), "the library freed it");
}

/// 戻り値（4-4）: `char*` は bytes、ほかのポインタは借用の Ptr、NULL は None、`bool` は真偽値。
#[test]
fn typed_pointer_returns() {
    let text = b"hey\0";
    let v = decode_typed_ret(&AbiTy::CStr { wide: false }, text.as_ptr() as u64).unwrap();
    assert!(matches!(v, Value::Bytes(b) if &*b == b"hey"));
    assert!(matches!(decode_typed_ret(&AbiTy::CStr { wide: false }, 0).unwrap(), Value::None));
    let v = decode_typed_ret(&AbiTy::PtrVal { elem: Arc::from("Db") }, 0x40).unwrap();
    assert!(matches!(&v, Value::Ptr(p) if p.elem_type() == "Db"));
    assert!(matches!(decode_typed_ret(&AbiTy::PtrVal { elem: Arc::from("Db") }, 0).unwrap(), Value::None));
    assert!(matches!(decode_typed_ret(&AbiTy::Bool, 1).unwrap(), Value::Bool(true)));
}

/// 数値への書き込みポインタに数値でない値が来たらエラー。数値どうしの食い違いは従来どおりハンドル経路へ。
#[test]
fn out_pointer_needs_a_number() {
    let mut ta = TypedArgs::new();
    let err = ta
        .marshal(&[Value::None], &[AbiTy::OutPtr { width: RawWidth::F64 }], &[])
        .err()
        .expect("must be an error");
    assert!(err.contains("must be a number") && err.contains("NoneType"), "{err}");

    let mut ta = TypedArgs::new();
    assert!(matches!(
        ta.marshal(&[Value::Float(1.5)], &[AbiTy::OutPtr { width: RawWidth::I32 }], &[]),
        Ok(Marshalled::TypeMismatch)
    ));
}

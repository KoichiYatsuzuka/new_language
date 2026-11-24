use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 呼ばれた回数を `ctx` の指す `AtomicUsize` に数える解放関数（テストごとに別の数え先を使う）。
unsafe extern "C" fn count_release(ctx: *mut std::ffi::c_void, address: u64) {
    assert_eq!(address, 0x1000);
    (*(ctx as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
}

fn owned(counter: &AtomicUsize) -> PtrHandle {
    let ctx = counter as *const AtomicUsize as usize;
    PtrHandle::new(0x1000, "int32", 0, Some(3), PtrOwner::Owned { release: count_release, ctx }, None).unwrap()
}

#[test]
fn drop_releases_once_after_the_last_copy() {
    let n = AtomicUsize::new(0);
    let a = Arc::new(owned(&n));
    let b = Arc::clone(&a);
    drop(a);
    assert_eq!(n.load(Ordering::SeqCst), 0, "a copy is still alive");
    drop(b);
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn explicit_release_is_not_repeated_by_drop() {
    let n = AtomicUsize::new(0);
    let p = owned(&n);
    p.release().unwrap();
    assert_eq!(n.load(Ordering::SeqCst), 1);
    assert!(p.release().unwrap_err().contains("already released"));
    assert!(p.address().is_err());
    assert!(p.display().contains("released"));
    drop(p);
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn invalidated_pointer_is_not_released() {
    let n = AtomicUsize::new(0);
    let p = owned(&n);
    p.invalidate();
    assert!(p.address().unwrap_err().contains("freed by the library"));
    assert!(p.release().is_err());
    drop(p);
    assert_eq!(n.load(Ordering::SeqCst), 0);
}

#[test]
fn borrowed_and_static_cannot_be_released() {
    let b = PtrHandle::new(0x1000, "void", 0, None, PtrOwner::Borrowed, None).unwrap();
    assert!(b.release().unwrap_err().contains("borrowed"));
    assert_eq!(b.address().unwrap(), 0x1000, "a failed release keeps the pointer usable");
    let s = PtrHandle::new(0x1000, "uint8", PTR_FLAG_STATIC, Some(0), PtrOwner::Static, None).unwrap();
    assert!(s.release().unwrap_err().contains("static"));
}

#[test]
fn invalid_construction_is_an_error() {
    assert!(PtrHandle::new(0, "int32", 0, Some(1), PtrOwner::Borrowed, None).unwrap_err().contains("NULL"));
    assert!(PtrHandle::new(1, "int32", 0, Some(-1), PtrOwner::Borrowed, None).is_err());
    // STATIC と書いていないのに解放しない（D-15）
    assert!(PtrHandle::new(1, "int32", 0, Some(1), PtrOwner::Static, None).is_err());
}

#[test]
fn display_never_shows_the_address() {
    let p = PtrHandle::new(0xdead_beef, "float64", 0, Some(2), PtrOwner::Borrowed, None).unwrap();
    assert_eq!(p.display(), "<Ptr[float64] len=2>");
    let q = PtrHandle::new(0xdead_beef, "Db", 0, None, PtrOwner::Borrowed, None).unwrap();
    assert_eq!(q.display(), "<Ptr[Db]>");
}

fn borrowed_over<T>(buf: &[T], elem: &str, length: Option<i64>, flags: u32) -> PtrHandle {
    PtrHandle::new(buf.as_ptr() as u64, elem, flags, length, PtrOwner::Borrowed, None).unwrap()
}

#[test]
fn to_list_reads_each_width() {
    use super::super::Value;
    let ints: [i32; 3] = [1, -2, 3];
    let got = borrowed_over(&ints, "int32", Some(3), 0).read_list().unwrap();
    assert!(matches!(got.as_slice(), [Value::Int(1), Value::Int(-2), Value::Int(3)]), "{got:?}");
    let bytes: [u8; 2] = [0xff, 7];
    let got = borrowed_over(&bytes, "int8", Some(2), 0).read_list().unwrap();
    assert!(matches!(got.as_slice(), [Value::Int(-1), Value::Int(7)]), "sign-extends int8: {got:?}");
    let floats: [f32; 1] = [0.5];
    let got = borrowed_over(&floats, "float32", Some(1), 0).read_list().unwrap();
    assert!(matches!(got.as_slice(), [Value::Float(f)] if *f == 0.5));
    let big: [u64; 1] = [u64::MAX];
    let e = borrowed_over(&big, "uint64", Some(1), 0).read_list().unwrap_err();
    assert!(e.starts_with("OverflowError"), "{e}");
}

#[test]
fn to_list_refuses_what_it_cannot_read() {
    let ints: [i32; 1] = [1];
    assert!(borrowed_over(&ints, "int32", None, 0).read_list().unwrap_err().contains("no length"));
    assert!(borrowed_over(&ints, "void", Some(1), 0).read_list().unwrap_err().contains("mustbe"));
    assert!(borrowed_over(&ints, "Db", Some(1), 0).read_list().unwrap_err().contains("not supported yet"));
}

#[test]
fn to_bytes_copies_character_data() {
    let text = *b"hi\0zz";
    assert_eq!(borrowed_over(&text, "uint8", Some(2), 0).read_bytes().unwrap(), b"hi");
    // 長さが無くても NUL 終端なら NUL の手前まで
    assert_eq!(borrowed_over(&text, "uint8", None, PTR_FLAG_NUL_TERMINATED).read_bytes().unwrap(), b"hi");
    let wide: [u16; 3] = [0x61, 0x3042, 0];
    assert_eq!(
        borrowed_over(&wide, "uint16", None, PTR_FLAG_NUL_TERMINATED).read_bytes().unwrap(),
        vec![0x61, 0x00, 0x42, 0x30]
    );
    assert!(borrowed_over(&text, "uint8", None, 0).read_bytes().is_err());
    let ints: [i32; 1] = [1];
    assert!(borrowed_over(&ints, "int32", Some(1), 0).read_bytes().unwrap_err().contains("character data"));
}

#[test]
fn annotation_matches_the_original_element_type() {
    use super::super::Value;
    let p = Value::Ptr(Arc::new(PtrHandle::new(1, "Db", 0, None, PtrOwner::Borrowed, None).unwrap()));
    assert_eq!(ptr_ann_matches("Ptr", &p), Some(true));
    assert_eq!(ptr_ann_matches("Ptr[void]", &p), Some(true));
    assert_eq!(ptr_ann_matches("Ptr[Db]", &p), Some(true));
    assert_eq!(ptr_ann_matches("Ptr[m.Db]", &p), Some(true));
    assert_eq!(ptr_ann_matches("Ptr[File]", &p), Some(false));
    assert_eq!(ptr_ann_matches("Ptr[int32]", &Value::Int(1)), Some(false));
    assert_eq!(ptr_ann_matches("list[int]", &p), None);
}

/// 共有ヘッダ・C# の写しの flags の値がこちらと同じ（4-1）。
#[test]
fn protocol_files_agree() {
    let header = include_str!("../../../../bridge/ptr_protocol/ar_ptr_protocol.h");
    let cs = include_str!("../../../../bridge/ptr_protocol/ArPtrProtocol.cs");
    // 利用者の C のコードに取り込まれるので ASCII だけ（MSVC は既定で CP932 として読み、UTF-8 のコメントが
    // 次の行を飲み込みうる・4-2 で直した）
    assert!(header.is_ascii(), "ar_ptr_protocol.h must be ASCII-only");
    let c_define = |name: &str| -> u32 {
        let line = header.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
        line.split_whitespace().nth(2).unwrap().trim_end_matches('u').parse().unwrap()
    };
    let cs_const = |name: &str| -> u32 {
        let key = format!("public const uint {name} = ");
        let line = cs.lines().find(|l| l.contains(&key)).expect(name);
        line.split(&key).nth(1).unwrap().trim_end_matches(';').trim().parse().unwrap()
    };
    assert_eq!(c_define("AR_PTR_CONST"), PTR_FLAG_CONST);
    assert_eq!(c_define("AR_PTR_NUL_TERMINATED"), PTR_FLAG_NUL_TERMINATED);
    assert_eq!(c_define("AR_PTR_STATIC"), PTR_FLAG_STATIC);
    assert_eq!(cs_const("Const"), PTR_FLAG_CONST);
    assert_eq!(cs_const("NulTerminated"), PTR_FLAG_NUL_TERMINATED);
    assert_eq!(cs_const("Static"), PTR_FLAG_STATIC);
    // 欄の並び（名前の順）も同じ
    let order = |text: &str, names: &[&str]| -> Vec<usize> { names.iter().map(|n| text.find(n).expect(n)).collect() };
    let c = order(header, &["uint64_t     address;", "elem_type;", "flags;", "_reserved;", "length;", "release;", "release_ctx;"]);
    let k = order(cs, &["ulong Address;", "ElemType;", "uint Flags;", "uint Reserved;", "long Length;", "> Release;", "ReleaseCtx;"]);
    assert!(c.windows(2).all(|w| w[0] < w[1]), "header field order");
    assert!(k.windows(2).all(|w| w[0] < w[1]), "C# field order");
}

/// 写しの並びが C のヘッダと同じ（64 ビットで 48 バイト・欄の位置）。
#[test]
fn protocol_layout_matches_the_header() {
    use std::mem::{offset_of, size_of};
    assert_eq!(size_of::<PtrProtocolRaw>(), 48);
    assert_eq!(offset_of!(PtrProtocolRaw, address), 0);
    assert_eq!(offset_of!(PtrProtocolRaw, elem_type), 8);
    assert_eq!(offset_of!(PtrProtocolRaw, flags), 16);
    assert_eq!(offset_of!(PtrProtocolRaw, _reserved), 20);
    assert_eq!(offset_of!(PtrProtocolRaw, length), 24);
    assert_eq!(offset_of!(PtrProtocolRaw, release), 32);
    assert_eq!(offset_of!(PtrProtocolRaw, release_ctx), 40);
}

fn raw(address: u64, elem: &'static std::ffi::CStr, flags: u32, length: i64, release: Option<ReleaseFn>) -> PtrProtocolRaw {
    PtrProtocolRaw {
        address,
        elem_type: elem.as_ptr(),
        flags,
        _reserved: 0,
        length,
        release,
        release_ctx: std::ptr::null_mut(),
    }
}

unsafe extern "C" fn noop_release(_ctx: *mut std::ffi::c_void, _address: u64) {}

/// 返された `ArPtrProtocol` の受け取り（4-5・D-12・D-15）。
#[test]
fn protocol_return_values() {
    use super::super::Value;
    let structs: Vec<Arc<str>> = vec![Arc::from("Point")];
    // 所有・STATIC・NULL
    let v = ptr_from_protocol(&raw(0x10, c"int32", 0, 3, Some(noop_release)), &structs).unwrap();
    assert!(matches!(&v, Value::Ptr(p) if p.elem_type() == "int32" && p.length() == Some(3)
        && matches!(p.owner(), PtrOwner::Owned { .. })));
    let v = ptr_from_protocol(&raw(0x10, c"uint8", PTR_FLAG_STATIC | PTR_FLAG_NUL_TERMINATED, -1, None), &structs)
        .unwrap();
    assert!(matches!(&v, Value::Ptr(p) if p.length().is_none() && matches!(p.owner(), PtrOwner::Static)));
    assert!(matches!(ptr_from_protocol(&raw(0, c"int32", 0, 0, None), &structs).unwrap(), Value::None));
    // 構造体はヘッダが宣言したものだけ
    assert!(ptr_from_protocol(&raw(0x10, c"Point", 0, 1, Some(noop_release)), &structs).is_ok());
    let e = ptr_from_protocol(&raw(0x10, c"Pont", 0, 1, Some(noop_release)), &structs).unwrap_err();
    assert!(e.contains("unknown element type 'Pont'"), "{e}");
    // 解放の責任が無い・長さが不正
    let e = ptr_from_protocol(&raw(0x10, c"int32", 0, 3, None), &structs).unwrap_err();
    assert!(e.contains("no release function") && e.contains("AR_PTR_STATIC"), "{e}");
    let e = ptr_from_protocol(&raw(0x10, c"int32", 0, -1, Some(noop_release)), &structs).unwrap_err();
    assert!(e.contains("-1 is allowed only with AR_PTR_NUL_TERMINATED"), "{e}");
}

/// Arrow の `list` / `bytes` を、呼び出しの間だけのコピーの `ArPtrProtocol` にする（4-7）。書き込み用なら書き戻す。
#[test]
fn lists_and_bytes_as_protocol_arguments() {
    use super::super::Value;
    use std::cell::RefCell;
    use std::rc::Rc;
    let elem = |a: &PtrProtocolArg| unsafe { std::ffi::CStr::from_ptr(a.raw.elem_type) }.to_str().unwrap().to_string();
    let a = protocol_arg(&Value::List(Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(-2)]))), false).unwrap();
    assert_eq!((elem(&a).as_str(), a.raw.length, a.raw.flags), ("int64", 2, PTR_FLAG_CONST));
    assert!(a.raw.release.is_none(), "Arrow keeps the memory; the library must not free it");
    let xs = unsafe { std::slice::from_raw_parts(a.raw.address as usize as *const i64, 2) };
    assert_eq!(xs, &[1, -2]);
    let f = protocol_arg(&Value::List(Rc::new(RefCell::new(vec![Value::Float(0.5)]))), false).unwrap();
    assert_eq!(elem(&f), "float64");
    let b = protocol_arg(&Value::bytes(b"hi".to_vec()), false).unwrap();
    assert_eq!((elem(&b).as_str(), b.raw.length), ("uint8", 2));
    // 型の決まらないもの・書き戻せないもの
    let empty = Value::List(Rc::new(RefCell::new(vec![])));
    assert!(protocol_arg(&empty, false).err().unwrap().contains("empty list"));
    let mixed = Value::List(Rc::new(RefCell::new(vec![Value::Int(1), Value::Float(2.0)])));
    assert!(protocol_arg(&mixed, false).err().unwrap().contains("element 1"));
    assert!(protocol_arg(&Value::bytes(b"x".to_vec()), true).err().unwrap().contains("mut list"));
    // 書き込み用: C が書いた中身を list へ戻す
    let list = Rc::new(RefCell::new(vec![Value::Int(1), Value::Int(2)]));
    let w = protocol_arg(&Value::List(list.clone()), true).unwrap();
    unsafe { *(w.raw.address as usize as *mut i64).add(1) = 40 };
    w.finish();
    assert!(matches!(list.borrow()[1], Value::Int(40)));
}

/// 宣言から要素の型が分かる呼び出し（cs-dll の配列・5-5）は、空の list を宣言の要素型の長さ 0 にする。
#[test]
fn empty_list_takes_the_declared_element_type() {
    use super::super::Value;
    let empty = Value::List(std::rc::Rc::new(std::cell::RefCell::new(vec![])));
    let a = protocol_arg_declared(&empty, false, "int32").unwrap();
    let elem = unsafe { std::ffi::CStr::from_ptr(a.raw.elem_type) }.to_str().unwrap().to_string();
    assert_eq!((elem.as_str(), a.raw.length), ("int32", 0));
    assert_ne!(a.raw.address, 0, "an empty array, not NULL");
    assert!(protocol_arg(&empty, false).is_err(), "without a declaration the element type is unknown");
}

#[test]
fn keeps_the_library_alive_while_any_copy_remains() {
    let lib: PtrKeepAlive = Arc::new(String::from("stand-in for a loaded DLL"));
    let weak = Arc::downgrade(&lib);
    let p = Arc::new(PtrHandle::new(0x1000, "void", 0, None, PtrOwner::Borrowed, Some(lib)).unwrap());
    let copy = Arc::clone(&p);
    drop(p);
    assert!(weak.upgrade().is_some(), "a copy still holds the library");
    drop(copy);
    assert!(weak.upgrade().is_none(), "the last copy lets the library go");
}

/// ポインタのフィールドを持つ C の構造体 1 つを読む（7-14）: 数値・`wchar_t*`（UTF-16LE の bytes）・構造体ポインタ
/// （借用の `Ptr`）・NULL（`None`）・`bool`
#[test]
fn reads_c_struct_fields_with_pointers() {
    use crate::interpreter::Value;
    #[repr(C)]
    struct Mixed {
        kind: i16,
        label: *const u16,
        rec: *const i32,
        extra: *const u8,
        flag: u8,
    }
    let wide: Vec<u16> = "ab".encode_utf16().chain([0]).collect();
    let target = 7i32;
    let m = Mixed { kind: -2, label: wide.as_ptr(), rec: &target, extra: std::ptr::null(), flag: 1 };
    let bytes = unsafe {
        std::slice::from_raw_parts(&m as *const Mixed as *const u8, std::mem::size_of::<Mixed>())
    };
    let layout = CStructLayout {
        fields: vec![
            (std::mem::offset_of!(Mixed, kind), CField::Num("int16".to_string())),
            (std::mem::offset_of!(Mixed, label), CField::Text { unit: 2 }),
            (std::mem::offset_of!(Mixed, rec), CField::Ptr("Rec".to_string())),
            (std::mem::offset_of!(Mixed, extra), CField::Ptr("void".to_string())),
            (std::mem::offset_of!(Mixed, flag), CField::Bool),
        ],
        size: std::mem::size_of::<Mixed>(),
    };
    let v = unsafe { read_c_struct_fields(bytes, &layout) }.unwrap();
    assert!(matches!(v[0], Value::Int(-2)), "{:?}", v[0]);
    assert!(matches!(&v[1], Value::Bytes(b) if b[..] == [b'a', 0, b'b', 0]), "{:?}", v[1]);
    assert!(
        matches!(&v[2], Value::Ptr(p) if p.elem_type() == "Rec" && matches!(p.owner(), PtrOwner::Borrowed)
            && p.address().unwrap() == &target as *const i32 as u64),
        "{:?}",
        v[2]
    );
    assert!(matches!(v[3], Value::None), "NULL is None: {:?}", v[3]);
    assert!(matches!(v[4], Value::Bool(true)), "{:?}", v[4]);
    // 足りないバイト列は誤り（読み越さない）
    assert!(unsafe { read_c_struct_fields(&bytes[..8], &layout) }.is_err());
}

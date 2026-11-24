use super::*;
use crate::cpp_header::types::IntWidth;

const I32: CType = CType::Int { width: IntWidth::W32, signed: true };

fn rec() -> CStructDef {
    CStructDef {
        name: "Rec".to_string(),
        fields: vec![("id".to_string(), I32), ("score".to_string(), CType::Double)],
        complete: true,
    }
}

/// 不完全な構造体（配列などのフィールドを読み飛ばした）。値で受け渡せない
/// 配列のフィールドを読み飛ばした構造体（`float m[4]` の `Mat2` など）。フィールドの並びが C と同じでない
fn incomplete() -> CStructDef {
    CStructDef { name: "Mat2".to_string(), fields: vec![], complete: false }
}

fn by_value(name: &str, ty: &str) -> CFnSig {
    let st = CType::ByValueStruct { type_name: ty.to_string() };
    CFnSig { name: name.to_string(), params: vec![("v".to_string(), st.clone())], ret: st, namespace: None, n_required: 1, defaults: vec![] }
}

/// 値渡し・値返しの構造体は `_Struct_` のまま受け渡す（アドレスを渡さない・戻り値をポインタとして読まない）。
#[test]
fn by_value_struct_is_passed_and_returned_by_value() {
    let defs = [rec()];
    let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
    let s = gen_dll_fn(&by_value("f", "Rec"), &map, &defs);
    assert!(s.contains("unsafe extern \"C\" fn(_Struct_Rec) -> _Struct_Rec"), "{s}");
    assert!(s.contains("let _v = _Struct_Rec {"), "{s}");
    assert!(s.contains("let _raw_r: _Struct_Rec = _fp(_v);"), "{s}");
    assert!(!s.contains("as *mut i8"), "{s}");
}

/// 生のポインタ（4-3）: `const char*` の引数は `bytes_arg`、`void*` の引数は `ptr_arg`、`char*` の戻り値は bytes、
/// ほかのポインタの戻り値は借用の `Ptr`。
#[test]
fn raw_pointer_parameters_and_returns() {
    let cstr = CType::CharPtr { wide: false, mutable: false };
    let s = gen_dll_fn(
        &CFnSig {
            name: "f".to_string(),
            params: vec![("s".to_string(), cstr.clone()), ("h".to_string(), CType::VoidPtr)],
            ret: cstr,
            namespace: None,
            n_required: 2,
        defaults: vec![],
        },
        &HashMap::new(),
        &[],
    );
    assert!(s.contains("((*CB).bytes_arg)(*argv.offset(0))"), "{s}");
    assert!(s.contains("((*CB).ptr_arg)(*argv.offset(1), b\"void\".as_ptr(), 4)"), "{s}");
    assert!(s.contains("_cstr_to_handle("), "{s}");
    let s = gen_dll_fn(
        &CFnSig { name: "g".to_string(), params: vec![], ret: CType::VoidPtr, namespace: None, n_required: 0, defaults: vec![] },
        &HashMap::new(),
        &[],
    );
    assert!(s.contains("((*CB).make_ptr)("), "{s}");
}

/// 型つき経路（4-4）: `bool`・`const char*`・`void*`・`ArPtrProtocol*` の引数と、`char*` の戻り値を純粋なキャストで渡す。
/// ハンドル経路の `ArPtrProtocol*` は `protocol_arg` で組み立て、呼び出しの後で `protocol_done` を呼ぶ。
#[test]
fn typed_wrapper_for_pointers_and_protocols() {
    let sig = CFnSig {
        name: "f".to_string(),
        params: vec![
            ("b".to_string(), CType::Bool),
            ("s".to_string(), CType::CharPtr { wide: false, mutable: false }),
            ("v".to_string(), CType::VoidPtr),
            ("p".to_string(), CType::PtrProtocol { by_pointer: true, mutable: true }),
        ],
        ret: CType::CharPtr { wide: false, mutable: false },
        namespace: None,
        n_required: 4,
    defaults: vec![],
    };
    assert!(cpp_typed_eligible(&sig, &[]));
    let s = gen_dll_fn(&sig, &HashMap::new(), &[]);
    assert!(s.contains("pub unsafe extern \"C\" fn f_typed("), "{s}");
    assert!(s.contains("let _b: u8 = (*_args.offset(0) != 0) as u8;"), "{s}");
    assert!(s.contains("let _s = (*_args.offset(1)) as usize as *const u8;"), "{s}");
    assert!(s.contains("let _v = (*_args.offset(2)) as usize as *mut i8;"), "{s}");
    assert!(s.contains("let _p = (*_args.offset(3)) as usize as *mut _ArPtrProtocol;"), "{s}");
    assert!(s.contains("*_ret = _fp(_b, _s, _v, _p) as usize as u64;"), "{s}");
    // ハンドル経路
    assert!(s.contains("((*CB).protocol_arg)(*argv.offset(3), &mut _p as *mut _ArPtrProtocol as *mut u8, 1);"), "{s}");
    assert!(s.contains("((*CB).protocol_done)(*argv.offset(3), _p.address);"), "{s}");
}

/// `ArPtrProtocol` の値返し（4-5）: 型つき経路は `*_ret` の指す領域へ書き、ハンドル経路は `protocol_ret` に構造体の名前と
/// 一緒に渡す。
#[test]
fn protocol_return_value() {
    let sig = CFnSig {
        name: "make".to_string(),
        params: vec![("n".to_string(), I32)],
        ret: CType::PtrProtocol { by_pointer: false, mutable: false },
        namespace: None,
        n_required: 1,
    defaults: vec![],
    };
    let defs = [rec()];
    assert!(cpp_typed_eligible(&sig, &defs));
    let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
    let s = gen_dll_fn(&sig, &map, &defs);
    assert!(s.contains("std::ptr::write((*_ret) as usize as *mut _ArPtrProtocol, _fp(_n));"), "{s}");
    assert!(s.contains("((*CB).protocol_ret)(&_r as *const _ArPtrProtocol as *const u8, b\"Rec\".as_ptr(), 3)"), "{s}");
}

/// `char*` の出力バッファ（4-7）: `bytes_buf_arg` でコピーを渡し、呼び出しの後で同じ長さの bytes として書き戻す。
#[test]
fn output_buffer_is_written_back() {
    let sig = CFnSig {
        name: "fill".to_string(),
        params: vec![("buf".to_string(), CType::CharPtr { wide: false, mutable: true }), ("n".to_string(), I32)],
        ret: CType::Void,
        namespace: None,
        n_required: 2,
    defaults: vec![],
    };
    let s = gen_dll_fn(&sig, &HashMap::new(), &[]);
    assert!(s.contains("let _buf = ((*CB).bytes_buf_arg)(*argv.offset(0), &mut _n_buf);"), "{s}");
    assert!(s.contains("((*CB).write_handle)(*argv.offset(0), ((*CB).make_bytes)(_buf, _n_buf));"), "{s}");
    assert!(!cpp_typed_eligible(&sig, &[]), "an output buffer goes through the handle path");
}

/// 引数を変換した後、エラーが設定されていれば C の関数を呼ばずに戻る。`bool` の引数は `to_bool`（7-12）。
#[test]
fn wrapper_returns_before_the_call_when_an_argument_fails() {
    let sig = CFnSig {
        name: "f".to_string(),
        params: vec![("n".to_string(), CType::Int { width: IntWidth::W32, signed: true }), ("b".to_string(), CType::Bool)],
        ret: CType::Int { width: IntWidth::W32, signed: true },
        namespace: None,
        n_required: 2,
    defaults: vec![],
    };
    let s = gen_dll_fn(&sig, &HashMap::new(), &[]);
    let check = s.find("((*CB).pending_error)() != 0").expect(&s);
    let call = s.find("_fp(").expect(&s);
    assert!(check < call, "{s}");
    assert!(s.contains("((*CB).to_bool)(*argv.offset(1)) as u8"), "{s}");
}

/// レイアウトが正確に分からない構造体（不完全・定義が無い）を値で受け渡す関数は生成しない。
#[test]
fn by_value_struct_without_exact_layout_is_not_generated() {
    let defs = [incomplete()];
    let map: HashMap<String, &CStructDef> = defs.iter().map(|d| (d.name.clone(), d)).collect();
    assert_eq!(gen_dll_fn(&by_value("f", "Mat2"), &map, &defs), "");
    assert_eq!(gen_dll_fn(&by_value("g", "Unknown"), &map, &defs), "");
}

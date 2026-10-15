// types.rs — C の型（`crate::cpp_header::types`）の、**実行時に依存する**メソッド。
//
// DLL の中継コードを生成するときの式（FFI の定数を使う）と、構造体の raw レイアウト（`RawLayout`）。
// 型の定義そのものは `crate::cpp_header::types` にある（拡張の wasm が取り込めるように切り出した・
// editor_import_resolution_plan.md 2-2）。

use super::super::native_api::{TL_FALSE, TL_NONE, TL_TRUE};
use crate::cpp_header::types::{CStructDef, CType, IntWidth};

/// C の整数の、中継コードでの Rust の型（`extern "C"` の宣言・構造体の写し・typed ABI の変換）。
///
/// 環境で幅が変わる `long` / `wchar_t` はここで決める（ffi_value_exchange_plan.md 1-6）: Windows（LLP64）は
/// `long` = 4 バイト・`wchar_t` = 符号なし 2 バイト、それ以外（LP64）は `long` = 8 バイト・`wchar_t` = 符号付き 4 バイト。
/// 正しい幅で宣言するので、戻り値の上位のゴミを読まず、符号付きは符号拡張・符号なしはゼロ拡張で `int` になる。
pub(crate) fn int_rust_type(width: IntWidth, signed: bool) -> &'static str {
    let win = cfg!(windows);
    match (width, signed) {
        (IntWidth::W8, true) => "i8",
        (IntWidth::W8, false) => "u8",
        (IntWidth::W16, true) => "i16",
        (IntWidth::W16, false) => "u16",
        (IntWidth::W32, true) => "i32",
        (IntWidth::W32, false) => "u32",
        (IntWidth::W64, true) => "i64",
        (IntWidth::W64, false) => "u64",
        (IntWidth::CLong, true) => if win { "i32" } else { "i64" },
        (IntWidth::CLong, false) => if win { "u32" } else { "u64" },
        (IntWidth::WChar, _) => if win { "u16" } else { "i32" },
    }
}

/// C の整数の、raw レイアウト・書き込みポインタでの幅の名前（`RawWidth::from_ann` が読む `int8` / `uint8` …）。
pub(crate) fn int_raw_ann(width: IntWidth, signed: bool) -> &'static str {
    match int_rust_type(width, signed) {
        "i8" => "int8",
        "u8" => "uint8",
        "i16" => "int16",
        "u16" => "uint16",
        "i32" => "int32",
        "u32" => "uint32",
        "i64" => "int64",
        _ => "uint64",
    }
}

impl CType {
    /// 生成されたラッパー内の `extern "C"` 宣言で使用する Rust 型文字列を返す。
    pub(crate) fn rust_extern_type(&self) -> String {
        match self {
            CType::Int { width, signed } => int_rust_type(*width, *signed).to_string(),
            CType::Float => "f32".to_string(),
            CType::Double => "f64".to_string(),
            // C++ の bool / C の _Bool は 1 バイト（1-6。以前は 4 バイトとして読み、上位のゴミを拾いえた）
            CType::Bool => "u8".to_string(),
            CType::Void => "()".to_string(),
            CType::VoidPtr => "*mut i8".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("*mut {}", inner.rust_extern_type())
                } else {
                    format!("*const {}", inner.rust_extern_type())
                }
            }
            CType::CharPtr { wide: false, mutable: true } => "*mut u8".to_string(),
            CType::CharPtr { wide: false, .. } => "*const u8".to_string(),
            CType::CharPtr { wide: true, .. } => "*const u16".to_string(),
            CType::OpaqueStructPtr { .. } => "*mut i8".to_string(),
            CType::FnPtr => "*mut i8".to_string(),
            // `_ArPtrProtocol` は中継コードの写し（4-4）。⚠ 戻り値（値返し）は 4-5 まで `unsupported_reason` が止める
            CType::PtrProtocol { by_pointer: false, .. } => "_ArPtrProtocol".to_string(),
            CType::PtrProtocol { by_pointer: true, mutable: true } => "*mut _ArPtrProtocol".to_string(),
            CType::PtrProtocol { by_pointer: true, mutable: false } => "*const _ArPtrProtocol".to_string(),
            // 値渡しの構造体は `_Struct_{name}` を値のまま使う（`gen_dll_fn` / `gen_dll_fn_typed` が直接書く・1-3）
            CType::ByValueStruct { .. } => panic!("by-value structs use their `_Struct_` mirror (gen_dll_fn)"),
        }
    }

    /// tl ハンドル (`i64`) をこの C 型（非ポインタ）に変換する Rust 式を返す。
    /// ポインタ型の場合は代わりに `gen_ptr_init` を使うこと。
    ///
    /// ⚠ 未対応の引数（`CFnSig::unsupported_reason`）の関数は生成しないので、`void*`・関数ポインタ・
    ///   `const char*` 以外の文字列はここへ来ない。来たら生成側の誤りなので止める（1-2。以前はハンドル番号を
    ///   そのままポインタとして渡すコードを生成していた）。
    pub(crate) fn from_handle(&self, handle: &str) -> String {
        match self {
            // 範囲外の値は C の幅に切り詰める（C ABI 型の既存の規約・c-abi-interop「C ABI 型の意味論」）
            CType::Int { width, signed } => {
                format!("((*CB).to_int)({handle}) as {}", int_rust_type(*width, *signed))
            }
            CType::Float => format!("((*CB).to_float)({handle}) as f32"),
            CType::Double => format!("((*CB).to_float)({handle})"),
            // `True` / `False` 以外はエラー（7-12。以前は `True` 以外を黙って偽にしていた）
            CType::Bool => format!("((*CB).to_bool)({handle}) as u8"),
            CType::Void => "()".to_string(),
            // 構造体ポインタ・`void*` は `Ptr`（要素型を照合）か `None`（NULL）を渡す（4-3・D-3・D-8）。⚠ 以前は
            // 構造体ポインタに Arrow 内部のハンドル番号をそのままポインタとして渡していた（落ちる・c-abi-interop P7 の
            // 残課題）。正確なレイアウトの構造体のインスタンスは typed 経路（`AbiTy::Ptr`）が受け渡すので、ここへ来ない
            // （来たら `ptr_arg` がエラーにする）。
            CType::OpaqueStructPtr { type_name, .. } => format!(
                "((*CB).ptr_arg)({handle}, b\"{type_name}\".as_ptr(), {}) as usize as *mut i8",
                type_name.len()
            ),
            CType::VoidPtr => format!("((*CB).ptr_arg)({handle}, b\"void\".as_ptr(), 4) as usize as *mut i8"),
            // `const char*` は `bytes` か `None`（4-3・D-13）
            CType::CharPtr { wide: false, mutable: false } => format!("((*CB).bytes_arg)({handle})"),
            CType::CharPtr { .. } | CType::FnPtr => {
                panic!("unsupported parameter type reached the wrapper generator (CFnSig::unsupported_reason)")
            }
            CType::ByValueStruct { .. } => panic!("by-value structs are built field by field (gen_dll_fn, 1-3)"),
            CType::Ptr { .. } => panic!("use gen_ptr_init for pointer parameters"),
            CType::PtrProtocol { .. } => {
                panic!("ArPtrProtocol parameter reached the wrapper generator (CFnSig::unsupported_reason)")
            }
        }
    }

    /// C の戻り値を tl ハンドルにラップする Rust 式を返す。
    ///
    /// ⚠ ポインタを返す関数は未対応（`CFnSig::unsupported_reason`）で生成しないので、ここへ来ない。来たら止める
    ///   （1-2。以前は `char*` / `T*` を黙って `None` に、`void*` / 構造体ポインタをハンドル番号として扱っていた）。
    pub(crate) fn to_handle(&self, val: &str) -> String {
        match self {
            // 符号付きは符号拡張、符号なしはゼロ拡張で i64 にする（`as i64` がそうなる）。符号なし 64 ビットの
            // 2^63 以上は `int` に収まらないので、黙って負にせず OverflowError にする（`_overflow_u64`・1-6）。
            CType::Int { width, signed } => match int_rust_type(*width, *signed) {
                "u64" => format!(
                    "{{ let _u: u64 = {val}; if _u > i64::MAX as u64 {{ _overflow_u64() }} else {{ ((*CB).make_int)(_u as i64) }} }}"
                ),
                _ => format!("((*CB).make_int)({val} as i64)"),
            },
            CType::Float => format!("((*CB).make_float)({val} as f64)"),
            CType::Double => format!("((*CB).make_float)({val})"),
            CType::Bool => format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}"),
            CType::Void => format!("{TL_NONE}i64"),
            // `char*` / `wchar_t*` は `bytes`（NUL 終端の借用をコピーする）、ほかのポインタは借用の `Ptr[要素]`。
            // NULL は `None`（4-3・D-7・D-8・D-9）。⚠ 以前はどれも使えない関数にしていた（1-2）。
            CType::CharPtr { wide: false, .. } => format!("_cstr_to_handle({val} as *const u8)"),
            CType::CharPtr { wide: true, .. } => format!("_wstr_to_handle({val} as *const u16)"),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } | CType::Ptr { .. } => {
                let elem = self.ptr_elem_name().expect("a pointer type has an element name");
                format!("((*CB).make_ptr)({val} as usize as u64, b\"{elem}\".as_ptr(), {})", elem.len())
            }
            CType::FnPtr => {
                panic!("unsupported return type reached the wrapper generator (CFnSig::unsupported_reason)")
            }
            CType::ByValueStruct { .. } => panic!("by-value struct returns are read field by field (gen_dll_fn, 1-3)"),
            CType::PtrProtocol { .. } => {
                panic!("ArPtrProtocol return reached the wrapper generator (CFnSig::unsupported_reason)")
            }
        }
    }

    /// [`Self::to_handle`] の式が例外（`TL_EXCEPTION`）を返しうるか。符号なし 64 ビットの 2^63 以上だけ（1-6）。
    /// ⚠ 戻り値そのもの以外（構造体のフィールド・書き込みポインタの書き戻し）に使うときは、生成コードがこれを見て
    ///   すぐ `TL_EXCEPTION` を返すこと。そうしないと例外のハンドルが値として使われ、呼び出しは正常に終わる。
    pub(crate) fn to_handle_may_raise(&self) -> bool {
        matches!(self, CType::Int { width, signed } if int_rust_type(*width, *signed) == "u64")
    }
}

impl CStructDef {
    /// この C/C++ 構造体の raw ブロックレイアウト（`InstanceData.raw` の C ABI 領域）を構築する。
    ///
    /// 条件: フィールドリストが完全（`complete`）で、全フィールドが幅の確定した
    /// プリミティブであること。整数は幅と符号どおり（`int8` 〜 `uint64`・1-6）/ `float`→f32 / `double`→f64。
    /// C の `long` / `wchar_t` は環境の幅で（`int_rust_type`）。
    /// `bool` は対象外（C++ で 1 バイトだが、Arrow のフィールドの型が bool のまま読み書きする手段が無い）。
    /// 対象かどうかの判定は `CStructDef::has_exact_layout`（型スタブ側の未対応判定と同じ定義・1-2）。
    pub fn raw_layout(&self) -> Option<crate::interpreter::value::RawLayout> {
        if !self.has_exact_layout() {
            return None;
        }
        let mut anns: Vec<(String, String)> = Vec::with_capacity(self.fields.len());
        for (name, ct) in &self.fields {
            let ann = match ct {
                CType::Int { width, signed } => int_raw_ann(*width, *signed),
                CType::Float => "float32",
                CType::Double => "float64",
                _ => return None,
            };
            anns.push((name.clone(), ann.to_string()));
        }
        crate::interpreter::value::RawLayout::from_fields(&anns)
    }

    /// ポインタのフィールドを持つ構造体の C のレイアウト（`Ptr[構造体].to_list()` で要素ごとに展開する・
    /// ffi_value_exchange_plan.md 7-14）。展開できなければ理由。
    ///
    /// フィールドの変換は値返しの構造体と同じ D-7 の規則（`codegen::field_to_handle`）: 数値・`bool` はそのまま、
    /// `char*` / `wchar_t*` は `bytes`、ほかのポインタは借用の `Ptr`、NULL は `None`。並びは C の規則（各フィールドを自分の
    /// 幅の境界へ・全体をいちばん大きい境界の倍数へ）で、ポインタはプロセスのポインタの幅。
    /// ⚠ `ArPtrProtocol` のフィールドは展開できない: 値返しでは 1 回だけ所有の `Ptr` にするが、配列の要素は `to_list()` を
    ///   呼ぶたびに読まれるので、同じメモリに持ち主が何人もできる（解放が二重になる）。
    pub fn c_struct_layout(&self, structs: &[CStructDef]) -> Result<crate::interpreter::value::CStructLayout, String> {
        use crate::interpreter::value::{CField, CStructLayout};
        if !self.complete || self.fields.is_empty() {
            return Err("its exact C layout is not known (a field was skipped: an array, a bit-field or a nested struct)"
                .to_string());
        }
        let ptr_size = std::mem::size_of::<usize>();
        let mut fields = Vec::with_capacity(self.fields.len());
        let (mut offset, mut align) = (0usize, 1usize);
        for (fname, ct) in &self.fields {
            let (size, field) = match ct {
                CType::Int { width, signed } => {
                    let ann = int_raw_ann(*width, *signed);
                    let w = crate::interpreter::value::RawWidth::from_ann(ann)
                        .map(|w| w.byte_width())
                        .expect("int_raw_ann gives a C width name");
                    (w, CField::Num(ann.to_string()))
                }
                CType::Float => (4, CField::Num("float32".to_string())),
                CType::Double => (8, CField::Num("float64".to_string())),
                CType::Bool => (1, CField::Bool),
                CType::CharPtr { wide: false, .. } => (ptr_size, CField::Text { unit: 1 }),
                // `wchar_t` の幅は環境で決まる（`int_rust_type`）
                CType::CharPtr { wide: true, .. } => (ptr_size, CField::Text { unit: if cfg!(windows) { 2 } else { 4 } }),
                CType::OpaqueStructPtr { type_name, .. } if !structs.iter().any(|d| &d.name == type_name) => {
                    return Err(format!(
                        "its field `{fname}` points to `{type_name}`, which the header does not declare as a struct (an unresolved typedef?)"
                    ))
                }
                CType::VoidPtr | CType::Ptr { .. } | CType::OpaqueStructPtr { .. } => {
                    (ptr_size, CField::Ptr(ct.ptr_elem_name().expect("a pointer type has an element name")))
                }
                CType::PtrProtocol { by_pointer: false, .. } => {
                    return Err(format!(
                        "its field `{fname}` is an `ArPtrProtocol`, which would get a new owner every time the array is expanded (return the structs one by one instead)"
                    ))
                }
                CType::PtrProtocol { by_pointer: true, .. } | CType::ByValueStruct { .. } | CType::FnPtr | CType::Void => {
                    return Err(format!(
                        "its field `{fname}` cannot be read from C memory (a nested struct, a function pointer or an `ArPtrProtocol*`)"
                    ))
                }
            };
            offset = offset.div_ceil(size) * size;
            fields.push((offset, field));
            offset += size;
            align = align.max(size);
        }
        Ok(CStructLayout { fields, size: offset.div_ceil(align) * align })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::CField;

    fn def(name: &str, fields: Vec<(&str, CType)>) -> CStructDef {
        CStructDef {
            name: name.to_string(),
            fields: fields.into_iter().map(|(n, t)| (n.to_string(), t)).collect(),
            complete: true,
        }
    }

    /// ポインタのフィールドを持つ構造体の並びが C（= Rust の `#[repr(C)]`）と同じになる（7-14）
    #[test]
    fn c_struct_layout_matches_repr_c() {
        #[repr(C)]
        struct Mixed {
            kind: i16,
            label: *const u16,
            rec: *const u8,
            extra: *const u8,
            flag: u8,
        }
        let rec = def("Rec", vec![("id", CType::Int { width: IntWidth::W32, signed: true })]);
        let mixed = def(
            "Mixed",
            vec![
                ("kind", CType::Int { width: IntWidth::W16, signed: true }),
                ("label", CType::CharPtr { wide: true, mutable: false }),
                ("rec", CType::OpaqueStructPtr { type_name: "Rec".to_string(), mutable: true }),
                ("extra", CType::VoidPtr),
                ("flag", CType::Bool),
            ],
        );
        let all = [rec, mixed.clone()];
        let l = mixed.c_struct_layout(&all).unwrap();
        let offsets: Vec<usize> = l.fields.iter().map(|(o, _)| *o).collect();
        assert_eq!(
            offsets,
            vec![
                std::mem::offset_of!(Mixed, kind),
                std::mem::offset_of!(Mixed, label),
                std::mem::offset_of!(Mixed, rec),
                std::mem::offset_of!(Mixed, extra),
                std::mem::offset_of!(Mixed, flag),
            ]
        );
        assert_eq!(l.size, std::mem::size_of::<Mixed>(), "the element stride includes the tail padding");
        assert_eq!(l.fields[0].1, CField::Num("int16".to_string()));
        assert_eq!(l.fields[1].1, CField::Text { unit: if cfg!(windows) { 2 } else { 4 } });
        assert_eq!(l.fields[2].1, CField::Ptr("Rec".to_string()));
        assert_eq!(l.fields[3].1, CField::Ptr("void".to_string()));
        assert_eq!(l.fields[4].1, CField::Bool);
    }

    /// 展開できない構造体は理由を返す（黙って読み違えない）
    #[test]
    fn c_struct_layout_reasons() {
        let i32t = CType::Int { width: IntWidth::W32, signed: true };
        let tagged = def("Tagged", vec![("tag", i32t.clone()), ("data", CType::PtrProtocol { by_pointer: false, mutable: false })]);
        assert!(tagged.c_struct_layout(&[]).unwrap_err().contains("`data` is an `ArPtrProtocol`"));
        let cb = def("Cb", vec![("f", CType::FnPtr)]);
        assert!(cb.c_struct_layout(&[]).unwrap_err().contains("`f` cannot be read"));
        let unknown = def("U", vec![("p", CType::OpaqueStructPtr { type_name: "TCHAR".to_string(), mutable: false })]);
        assert!(unknown.c_struct_layout(&[]).unwrap_err().contains("does not declare as a struct"));
        let mut partial = def("P", vec![("x", i32t)]);
        partial.complete = false;
        assert!(partial.c_struct_layout(&[]).unwrap_err().contains("not known"));
    }
}

// types.rs — C の型（`crate::cpp_header::types`）の、**実行時に依存する**メソッド。
//
// DLL の中継コードを生成するときの式（FFI の定数を使う）と、構造体の raw レイアウト（`RawLayout`）。
// 型の定義そのものは `crate::cpp_header::types` にある（拡張の wasm が取り込めるように切り出した・
// editor_import_resolution_plan.md 2-2）。

use super::super::native_api::{TL_FALSE, TL_NONE, TL_TRUE};
use crate::cpp_header::types::{CStructDef, CType};

impl CType {
    /// 生成されたラッパー内の `extern "C"` 宣言で使用する Rust 型文字列を返す。
    pub(crate) fn rust_extern_type(&self) -> String {
        match self {
            CType::Int => "i32".to_string(),
            CType::Long => "i64".to_string(),
            CType::Float => "f32".to_string(),
            CType::Double => "f64".to_string(),
            CType::Bool => "i32".to_string(),
            CType::Void => "()".to_string(),
            CType::VoidPtr => "*mut i8".to_string(),
            CType::Ptr { inner, mutable } => {
                if *mutable {
                    format!("*mut {}", inner.rust_extern_type())
                } else {
                    format!("*const {}", inner.rust_extern_type())
                }
            }
            CType::CharPtr { wide: false, .. } => "*const u8".to_string(),
            CType::CharPtr { wide: true, .. } => "*const u16".to_string(),
            CType::OpaqueStructPtr { .. } => "*mut i8".to_string(),
            CType::FnPtr => "*mut i8".to_string(),
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
            CType::Int => format!("((*CB).to_int)({handle}) as i32"),
            CType::Long => format!("((*CB).to_int)({handle})"),
            CType::Float => format!("((*CB).to_float)({handle}) as f32"),
            CType::Double => format!("((*CB).to_float)({handle})"),
            CType::Bool => format!("if {handle} == {TL_TRUE}i64 {{ 1i32 }} else {{ 0i32 }}"),
            CType::Void => "()".to_string(),
            // 構造体ポインタは typed 経路で受け渡す（`AbiTy::Ptr`）。typed 経路に合わない引数はハンドル経路へ
            // 落とさずエラーにする（`TypedArgs::marshal`・1-2）ので、ここは typed の関数が見つからないときだけ。
            CType::OpaqueStructPtr { .. } => format!("{handle} as *mut i8"),
            CType::CharPtr { wide: false, mutable: false } => format!("((*CB).to_cstr)({handle})"),
            CType::CharPtr { .. } | CType::VoidPtr | CType::FnPtr => {
                panic!("unsupported parameter type reached the wrapper generator (CFnSig::unsupported_reason)")
            }
            CType::ByValueStruct { .. } => panic!("by-value structs are built field by field (gen_dll_fn, 1-3)"),
            CType::Ptr { .. } => panic!("use gen_ptr_init for pointer parameters"),
        }
    }

    /// C の戻り値を tl ハンドルにラップする Rust 式を返す。
    ///
    /// ⚠ ポインタを返す関数は未対応（`CFnSig::unsupported_reason`）で生成しないので、ここへ来ない。来たら止める
    ///   （1-2。以前は `char*` / `T*` を黙って `None` に、`void*` / 構造体ポインタをハンドル番号として扱っていた）。
    pub(crate) fn to_handle(&self, val: &str) -> String {
        match self {
            CType::Int => format!("((*CB).make_int)({val} as i64)"),
            CType::Long => format!("((*CB).make_int)({val})"),
            CType::Float => format!("((*CB).make_float)({val} as f64)"),
            CType::Double => format!("((*CB).make_float)({val})"),
            CType::Bool => format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}"),
            CType::Void => format!("{TL_NONE}i64"),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } | CType::FnPtr | CType::CharPtr { .. } | CType::Ptr { .. } => {
                panic!("unsupported return type reached the wrapper generator (CFnSig::unsupported_reason)")
            }
            CType::ByValueStruct { .. } => panic!("by-value struct returns are read field by field (gen_dll_fn, 1-3)"),
        }
    }
}

impl CStructDef {
    /// この C/C++ 構造体の raw ブロックレイアウト（`InstanceData.raw` の C ABI 領域）を構築する。
    ///
    /// 条件: フィールドリストが完全（`complete`）で、全フィールドが幅の確定した
    /// プリミティブであること。C の `int`→i32 / `float`→f32 / `double`→f64。
    /// C の `long` は環境依存幅（Windows LLP64=4B / Linux LP64=8B）のため対象外。
    /// `bool` は C++ で 1 バイトだが既存ミラーが i32 を仮定しており曖昧なため対象外。
    /// 対象かどうかの判定は `CStructDef::has_exact_layout`（型スタブ側の未対応判定と同じ定義・1-2）。
    pub fn raw_layout(&self) -> Option<crate::interpreter::value::RawLayout> {
        if !self.has_exact_layout() {
            return None;
        }
        let mut anns: Vec<(String, String)> = Vec::with_capacity(self.fields.len());
        for (name, ct) in &self.fields {
            let ann = match ct {
                CType::Int => "int32",
                CType::Float => "float32",
                CType::Double => "float64",
                _ => return None,
            };
            anns.push((name.clone(), ann.to_string()));
        }
        crate::interpreter::value::RawLayout::from_fields(&anns)
    }
}

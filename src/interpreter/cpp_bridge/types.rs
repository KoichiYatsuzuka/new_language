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
            CType::CharPtr => "*const u8".to_string(),
            CType::OpaqueStructPtr { .. } => "*mut i8".to_string(),
            CType::ByValueStruct { .. } | CType::FnPtr => "*mut i8".to_string(),
        }
    }

    /// tl ハンドル (`i64`) をこの C 型（非ポインタ）に変換する Rust 式を返す。
    /// ポインタ型の場合は代わりに `gen_ptr_init` を使うこと。
    pub(crate) fn from_handle(&self, handle: &str) -> String {
        match self {
            CType::Int => format!("((*CB).to_int)({handle}) as i32"),
            CType::Long => format!("((*CB).to_int)({handle})"),
            CType::Float => format!("((*CB).to_float)({handle}) as f32"),
            CType::Double => format!("((*CB).to_float)({handle})"),
            CType::Bool => format!("if {handle} == {TL_TRUE}i64 {{ 1i32 }} else {{ 0i32 }}"),
            CType::Void => "()".to_string(),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } => format!("{handle} as *mut i8"),
            CType::CharPtr => format!("((*CB).to_cstr)({handle})"),
            CType::ByValueStruct { .. } | CType::FnPtr => format!("{handle} as *mut i8"),
            CType::Ptr { .. } => panic!("use gen_ptr_init for pointer parameters"),
        }
    }

    /// C の戻り値を tl ハンドルにラップする Rust 式を返す。
    pub(crate) fn to_handle(&self, val: &str) -> String {
        match self {
            CType::Int => format!("((*CB).make_int)({val} as i64)"),
            CType::Long => format!("((*CB).make_int)({val})"),
            CType::Float => format!("((*CB).make_float)({val} as f64)"),
            CType::Double => format!("((*CB).make_float)({val})"),
            CType::Bool => format!("if {val} != 0 {{ {TL_TRUE}i64 }} else {{ {TL_FALSE}i64 }}"),
            CType::Void => format!("{TL_NONE}i64"),
            CType::VoidPtr | CType::OpaqueStructPtr { .. } => format!("{val} as i64"),
            // ByValueStruct: val is *mut i8 pointing at the shim's static buffer
            CType::ByValueStruct { .. } | CType::FnPtr => format!("{val} as i64"),
            CType::CharPtr | CType::Ptr { .. } => format!("{TL_NONE}i64"), // pointers as return are opaque
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
    pub fn raw_layout(&self) -> Option<crate::interpreter::value::RawLayout> {
        if !self.complete {
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

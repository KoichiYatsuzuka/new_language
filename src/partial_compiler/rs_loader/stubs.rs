// rs_loader/stubs.rs — 型スタブ生成: 解析したシグネチャから Arrow の Stmt(スタブ)を組み立てる make_stubs。

#[allow(unused_imports)]
use {
    std::collections::HashMap, std::path::{Path, PathBuf},
    crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt},
    crate::partial_compiler::llvm_codegen::FnExport,
    crate::partial_compiler::module_compiler::{cache_native, native_lib_ext},
};
#[allow(unused_imports)]
use super::*;

// ── Stub generation ───────────────────────────────────────────────────────────

pub(crate) fn make_stubs(fns: &[RsFnSig], structs: &[RsStructSig]) -> Vec<Stmt> {
    let mut stmts: Vec<Stmt> = Vec::new();

    // Free function stubs
    // 値引数は常に不変（`&mut T` 引数を持つ関数は is_abi_compatible で除外済み —
    // もし将来対応する場合は writable_ref=true で `mut` マーキングすること。Param::bridge 参照）。
    for sig in fns {
        let params: Vec<Param> = sig.params.iter()
            .map(|p| Param::bridge(&p.name, Some(rust_type_to_ar(&p.rust_type).to_string()), false))
            .collect();
        stmts.push(Stmt::FnDef {
            name: sig.name.clone(),
            template_params: vec![],
            params,
            return_type: sig.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string()),
            body: vec![],
            is_abstract: true,
            is_static: false,
            is_class_method: false,
            decorators: vec![],
            access: Accessibility::Public,
        });
    }

    // Class stubs for each struct
    for st in structs {
        let mut class_body: Vec<Stmt> = Vec::new();

        // Internal handle field (private, mutable, default 0)
        class_body.push(Stmt::Field {
            name: "__rs_handle__".to_string(),
            kind: FieldKind::Mut,
            type_ann: "int".to_string(),
            default: Some(Expr::Int(0)),
            access: Accessibility::Public,
        });

        // Public fields mirroring the Rust struct
        for field in &st.fields {
            class_body.push(Stmt::Field {
                name: field.name.clone(),
                kind: FieldKind::Mut,
                type_ann: rust_type_to_ar(&field.rust_type).to_string(),
                default: None,
                access: Accessibility::Public,
            });
        }

        // __init__ stub: (mut self, ctor_param0, ctor_param1, ...)
        {
            let mut params = vec![Param {
                name: "self".to_string(),
                mutable: true,
                type_ann: None,
                default: None,
                variadic: false,
            }];
            for cp in &st.ctor_params {
                params.push(Param {
                    name: cp.name.clone(),
                    mutable: false,
                    type_ann: Some(rust_type_to_ar(&cp.rust_type).to_string()),
                    default: None,
                    variadic: false,
                });
            }
            class_body.push(Stmt::FnDef {
                name: "__init__".to_string(),
                template_params: vec![],
                params,
                return_type: None,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // drop stub
        class_body.push(Stmt::FnDef {
            name: "drop".to_string(),
            template_params: vec![],
            params: vec![Param { name: "self".to_string(), mutable: true, type_ann: None, default: None, variadic: false }],
            return_type: None,
            body: vec![],
            is_abstract: true,
            is_static: false,
            is_class_method: false,
            decorators: vec![],
            access: Accessibility::Public,
        });

        // Field getter stubs: get_{field}(let self) -> T
        for field in &st.fields {
            let getter_name = format!("get_{}", field.name);
            class_body.push(Stmt::FnDef {
                name: getter_name,
                template_params: vec![],
                params: vec![Param { name: "self".to_string(), mutable: false, type_ann: None, default: None, variadic: false }],
                return_type: Some(rust_type_to_ar(&field.rust_type).to_string()),
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // Field setter stubs: set_{field}(mut self, val: T)
        for field in &st.fields {
            let setter_name = format!("set_{}", field.name);
            class_body.push(Stmt::FnDef {
                name: setter_name,
                template_params: vec![],
                params: vec![
                    Param { name: "self".to_string(), mutable: true, type_ann: None, default: None, variadic: false },
                    Param { name: "val".to_string(), mutable: false, type_ann: Some(rust_type_to_ar(&field.rust_type).to_string()), default: None, variadic: false },
                ],
                return_type: None,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // Method stubs
        // `&mut self` レシーバは Arrow の `mut self` として型検査される（Param::bridge）。
        // 値引数の `&mut T` は is_abi_compatible が拒否するため常に不変。
        for m in &st.methods {
            let mut params = vec![Param::bridge("self", None, m.self_mutable)];
            for p in &m.params {
                params.push(Param::bridge(
                    &p.name,
                    Some(rust_type_to_ar(&p.rust_type).to_string()),
                    false,
                ));
            }
            // Return type: primitive or a struct class name
            let ret_type_str = m.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string())
                .or_else(|| m.return_struct.clone());
            class_body.push(Stmt::FnDef {
                name: m.name.clone(),
                template_params: vec![],
                params,
                return_type: ret_type_str,
                body: vec![],
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        stmts.push(Stmt::ClassDef {
            name: st.name.clone(),
            template_params: vec![],
            bases: vec![],
            decorators: vec![],
            body: class_body,
        });
    }

    stmts
}


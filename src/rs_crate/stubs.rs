// rs_crate/stubs.rs — 型スタブ生成: 解析したシグネチャから Arrow の Stmt(スタブ)を組み立てる make_stubs。

use crate::ast::{Accessibility, Expr, FieldKind, Param, Stmt};
use super::*;

// ── Stub generation ───────────────────────────────────────────────────────────

/// 未対応の関数・メソッドの本体に置く印（`FFI_UNSUPPORTED_MARK` で始まる docstring・2-5）。型検査が読んで、使った箇所を
/// 静的エラーにする（`type_check::ffi_unsupported_reason`）。C# のスタブと同じくデコレータは使わない（クラスとして
/// 実行されるとデコレータが適用される）。
fn unsupported_body(reason: &str) -> Vec<Stmt> {
    vec![Stmt::Expr(Expr::Str(format!("{}{reason}", crate::cpp_header::FFI_UNSUPPORTED_MARK).into()))]
}

/// 未対応の関数・メソッドの型スタブ（引数・戻り値の型は分かる範囲で。分からなければ `Any`）。
fn unsupported_fn_stub(u: &RsUnsupported, receiver: Option<bool>) -> Stmt {
    let mut params: Vec<Param> = match receiver {
        Some(mutable) => vec![Param::bridge("self", None, mutable)],
        None => vec![],
    };
    params.extend(u.params.iter().map(|p| Param::bridge(&p.name, Some(rust_param_type_to_ar(&p.rust_type)), false)));
    Stmt::FnDef { src: None,
        name: u.name.clone(),
        template_params: vec![],
        params,
        return_type: Some(u.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string()).unwrap_or_else(|| "None".to_string())),
        body: unsupported_body(&u.reason),
        is_abstract: true,
        is_static: false,
        is_class_method: false,
        decorators: vec![],
        access: Accessibility::Public,
    }
}

pub(crate) fn make_stubs(fns: &[RsFnSig], structs: &[RsStructSig], unsupported: &[RsUnsupported]) -> Vec<Stmt> {
    let mut stmts: Vec<Stmt> = Vec::new();

    // 今の Arrow から呼べない関数（2-5）。以前は import 時に黙って消えていた。
    for u in unsupported {
        stmts.push(unsupported_fn_stub(u, None));
    }

    // Free function stubs
    // 値引数は常に不変（`&mut T` 引数を持つ関数は `is_param_type` が拒否し、未対応として印付きで載る —
    // もし将来対応する場合は writable_ref=true で `mut` マーキングすること。Param::bridge 参照）。
    for sig in fns {
        let params: Vec<Param> = sig.params.iter()
            .map(|p| Param::bridge(&p.name, Some(rust_param_type_to_ar(&p.rust_type)), false))
            .collect();
        stmts.push(Stmt::FnDef { src: None,
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
        class_body.push(Stmt::Field { src: None,
            name: "__rs_handle__".to_string(),
            kind: FieldKind::Mut,
            type_ann: "int".to_string(),
            default: Some(Expr::Int(0)),
            access: Accessibility::Public,
        });

        // Public fields mirroring the Rust struct
        for field in &st.fields {
            class_body.push(Stmt::Field { src: None,
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
                    type_ann: Some(rust_param_type_to_ar(&cp.rust_type)),
                    default: None,
                    variadic: false,
                });
            }
            class_body.push(Stmt::FnDef { src: None,
                name: "__init__".to_string(),
                template_params: vec![],
                params,
                return_type: None,
                // Arrow から構築できない構造体（2-5）。メソッドの戻り値として受け取った値は使える。
                body: st.ctor_unsupported.as_deref().map(unsupported_body).unwrap_or_default(),
                is_abstract: true,
                is_static: false,
                is_class_method: false,
                decorators: vec![],
                access: Accessibility::Public,
            });
        }

        // drop stub
        class_body.push(Stmt::FnDef { src: None,
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
            class_body.push(Stmt::FnDef { src: None,
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
            class_body.push(Stmt::FnDef { src: None,
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
        // 値引数の `&mut T` は `is_param_type` が拒否するため常に不変（未対応のメソッドとして載る）。
        for m in &st.methods {
            let mut params = vec![Param::bridge("self", None, m.self_mutable)];
            for p in &m.params {
                params.push(Param::bridge(
                    &p.name,
                    Some(rust_param_type_to_ar(&p.rust_type)),
                    false,
                ));
            }
            // Return type: primitive or a struct class name
            let ret_type_str = m.return_type.as_deref().map(|r| rust_type_to_ar(r).to_string())
                .or_else(|| m.return_struct.clone());
            class_body.push(Stmt::FnDef { src: None,
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

        // 今の Arrow から呼べないメソッド（2-5）
        for u in &st.unsupported_methods {
            class_body.push(unsupported_fn_stub(u, Some(false)));
        }

        stmts.push(Stmt::ClassDef { src: None,
            name: st.name.clone(),
            template_params: vec![],
            bases: vec![],
            // ⚠ Rust の構造体スタブは trait を継承しない（タスク 9.9）。
            base_args: vec![],
            decorators: vec![],
            body: class_body,
        });
    }

    stmts
}


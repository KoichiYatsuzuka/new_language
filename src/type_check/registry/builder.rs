// type_check/registry/builder.rs — 収集パス。AST を先行スキャンして `TypeRegistry` を組み立てる。
//
// Phase 5A-3b で `TypeChecker::collect_fn_sigs` をここへ移設したもの。
// **レジストリへ書き込めるのはこのファイルだけ**であり、`build()` を通した後は
// 読み取り専用の `TypeRegistry` になる（registry/mod.rs の不変条件）。
//
// ここでは診断（エラー・警告）を一切報告しない。収集パスは「宣言を索引化する」
// だけの責務で、`Diagnostics` に依存しない（依存グラフを一方向に保つため）。

use std::collections::{HashMap, HashSet};

use crate::ast::{Accessibility, FieldKind, Stmt};

use super::super::types::{FnSig, InferredType, ProtocolField, ProtocolInfo, ProtocolMethod};
use super::TypeRegistry;

/// 組み込みで登録される例外クラス名。`TypeChecker::new` がグローバルスコープの
/// 束縛を作る際にも使うため公開している。
pub(in crate::type_check) const EXCEPTION_CLASS_NAMES: [&str; 17] = [
    "Exception",
    "ValueError",
    "TypeError",
    "NameError",
    "AttributeError",
    "IndexError",
    "KeyError",
    "ZeroDivisionError",
    "RuntimeError",
    "StopIteration",
    "NotImplementedError",
    "OverflowError",
    "IOError",
    "OSError",
    "AssertionError",
    "ArithmeticError",
    "AccessError",
];

/// 組み込みで登録される new_type（型名 → 元のプリミティブ型名）。
const BUILTIN_NEW_TYPES: [(&str, &str); 3] = [("path", "str"), ("Index", "int"), ("Size", "int")];

/// `TypeRegistry` の構築器。`collect` で AST を走査し、`build` で凍結する。
pub(in crate::type_check) struct TypeRegistryBuilder {
    reg: TypeRegistry,
}

impl TypeRegistryBuilder {
    /// 組み込みクラス（例外クラス・`path`/`Index`/`Size`・`slice`）を登録した状態で生成する。
    pub(in crate::type_check) fn with_builtins() -> Self {
        let mut known_class_names: HashSet<String> = HashSet::new();
        let mut new_type_originals: HashMap<String, String> = HashMap::new();
        for (cls_name, prim_type) in BUILTIN_NEW_TYPES {
            known_class_names.insert(cls_name.to_string());
            new_type_originals.insert(cls_name.to_string(), prim_type.to_string());
        }
        known_class_names.insert("slice".to_string());

        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        for class_name in EXCEPTION_CLASS_NAMES {
            known_class_names.insert(class_name.to_string());
            class_bases.insert(class_name.to_string(), vec!["Error".to_string()]);
        }

        Self {
            reg: TypeRegistry {
                fn_sigs: HashMap::new(),
                class_method_sigs: HashMap::new(),
                trait_method_sigs: HashMap::new(),
                trait_field_details: HashMap::new(),
                known_class_names,
                new_type_originals,
                class_bases,
                class_fields: HashMap::new(),
                class_field_details: HashMap::new(),
                class_member_access: HashMap::new(),
                class_static_methods: HashMap::new(),
                known_protocols: HashMap::new(),
            },
        }
    }

    /// 収集を終えてレジストリを凍結する。
    pub(in crate::type_check) fn build(self) -> TypeRegistry {
        self.reg
    }

    /// `NamedInstance(name)` がプロトコル名であれば `Protocol(name)` に変換する。
    /// 収集パス中は `known_protocols` が育っている途中なので、ここで参照するのは
    /// 「その時点までに登録済みのプロトコル」であることに注意（移設前の挙動と同一）。
    fn resolve_protocol_type(&self, ty: InferredType) -> InferredType {
        if let InferredType::NamedInstance(ref name) = ty {
            if self.reg.known_protocols.contains_key(name.as_str()) {
                return InferredType::Protocol(name.clone());
            }
        }
        ty
    }

    /// 文のスライスを先行スキャンして関数・クラス・trait のシグネチャ情報を収集する。
    pub(in crate::type_check) fn collect(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::FnDef {
                    name,
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    let variadic_param = params.iter().find(|p| p.variadic);
                    let sig = FnSig {
                        params: params
                            .iter()
                            .filter(|p| !p.variadic)
                            .map(|p| {
                                let ty = p.type_ann.as_deref()
                                    .and_then(InferredType::from_ann)
                                    .map(|t| self.resolve_protocol_type(t));
                                (p.name.clone(), ty)
                            })
                            .collect(),
                        required_count: params
                            .iter()
                            .filter(|p| !p.variadic && p.default.is_none())
                            .count(),
                        return_type: return_type.as_deref()
                            .and_then(InferredType::from_ann)
                            .map(|t| self.resolve_protocol_type(t)),
                        variadic_type: variadic_param
                            .and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann)),
                    };
                    self.reg.fn_sigs.entry(name.clone()).or_default().push(sig);
                    self.collect(body);
                }
                Stmt::ClassDef {
                    name, bases, body, ..
                } => {
                    self.reg.known_class_names.insert(name.clone());
                    self.reg.class_bases.insert(name.clone(), bases.clone());
                    self.collect_class_methods(name, body);
                    self.collect_class_members(name, body);
                    // Only recurse into method bodies for nested closures;
                    // class methods themselves must NOT be added to fn_sigs.
                    for s in body.iter() {
                        if let Stmt::FnDef { body: method_body, .. } = s {
                            self.collect(method_body);
                        }
                    }
                }
                Stmt::EnumDef { name, .. } => {
                    self.reg.known_class_names.insert(name.clone());
                    let item_type_name = format!("enum_item_{}", name);
                    self.reg.known_class_names.insert(item_type_name);
                }
                Stmt::TraitDef { name, body, .. } => {
                    self.collect_trait(name, body);
                }
                Stmt::ProtocolDef { name, body } => {
                    self.collect_protocol(name, body);
                }
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        self.collect(&arm.body);
                    }
                }
                Stmt::If {
                    branches,
                    else_body,
                } => {
                    for (_, body) in branches {
                        self.collect(body);
                    }
                    if let Some(body) = else_body {
                        self.collect(body);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    self.collect(body);
                }
                _ => {}
            }
        }
        for stmt in stmts {
            if let Stmt::NewTypeDef { name, original } = stmt {
                self.reg.known_class_names.insert(name.clone());
                self.reg
                    .new_type_originals
                    .insert(name.clone(), original.clone());
                if let Some(orig_sigs) = self.reg.class_method_sigs.get(original).cloned() {
                    self.reg.class_method_sigs.insert(name.clone(), orig_sigs);
                }
            }
        }
    }

    /// クラス本体のメソッドシグネチャを収集して `class_method_sigs` に登録する。
    fn collect_class_methods(&mut self, name: &str, body: &[Stmt]) {
        let mut cls_methods: HashMap<String, Vec<FnSig>> = HashMap::new();
        for s in body.iter() {
            if let Stmt::FnDef {
                name: mname,
                template_params,
                params,
                return_type,
                ..
            } = s
            {
                let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                    format!("__cast__[{}]", template_params[0].name)
                } else {
                    mname.clone()
                };
                let variadic_param = params.iter().find(|p| p.variadic);
                let sig = FnSig {
                    params: params
                        .iter()
                        .filter(|p| !p.variadic)
                        .map(|p| {
                            (
                                p.name.clone(),
                                p.type_ann.as_deref().and_then(InferredType::from_ann),
                            )
                        })
                        .collect(),
                    required_count: params
                        .iter()
                        .filter(|p| !p.variadic && p.default.is_none())
                        .count(),
                    return_type: return_type
                        .as_deref()
                        .and_then(InferredType::from_ann),
                    variadic_type: variadic_param.and_then(|p| {
                        p.type_ann.as_deref().and_then(InferredType::from_ann)
                    }),
                };
                cls_methods.entry(storage_name).or_default().push(sig);
            }
        }
        self.reg.class_method_sigs.insert(name.to_string(), cls_methods);
    }

    /// クラス本体のフィールド・アクセス指定・スタティックメソッドを収集する。
    fn collect_class_members(&mut self, name: &str, body: &[Stmt]) {
        let mut fields: HashMap<String, bool> = HashMap::new();
        let mut field_details: HashMap<String, (FieldKind, InferredType)> = HashMap::new();
        let mut member_access: HashMap<String, Accessibility> = HashMap::new();
        let mut static_methods: HashSet<String> = HashSet::new();
        for s in body.iter() {
            match s {
                Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    access,
                    ..
                } => {
                    let mutable = matches!(kind, FieldKind::Mut);
                    fields.insert(fname.clone(), mutable);
                    let fty = InferredType::from_ann(type_ann)
                        .unwrap_or(InferredType::Unresolved);
                    field_details.insert(fname.clone(), (kind.clone(), fty));
                    if *access != Accessibility::Public {
                        member_access.insert(fname.clone(), access.clone());
                    }
                }
                Stmt::FnDef {
                    name: mname,
                    is_static,
                    access,
                    ..
                } => {
                    if *access != Accessibility::Public {
                        member_access.insert(mname.clone(), access.clone());
                    }
                    if *is_static {
                        static_methods.insert(mname.clone());
                    }
                }
                _ => {}
            }
        }
        self.reg.class_fields.insert(name.to_string(), fields);
        self.reg.class_field_details.insert(name.to_string(), field_details);
        self.reg.class_member_access.insert(name.to_string(), member_access);
        if !static_methods.is_empty() {
            self.reg
                .class_static_methods
                .insert(name.to_string(), static_methods);
        }
    }

    /// trait 本体のメソッドシグネチャ・フィールドを収集する（Intersection 適合チェック用）。
    fn collect_trait(&mut self, name: &str, body: &[Stmt]) {
        let mut tmethods: HashMap<String, Vec<FnSig>> = HashMap::new();
        let mut tfields: HashMap<String, (FieldKind, InferredType)> = HashMap::new();
        for s in body.iter() {
            match s {
                Stmt::FnDef { name: mname, params, return_type, body: method_body, .. } => {
                    let variadic_param = params.iter().find(|p| p.variadic);
                    let sig = FnSig {
                        params: params
                            .iter()
                            .filter(|p| !p.variadic)
                            .map(|p| (p.name.clone(), p.type_ann.as_deref().and_then(InferredType::from_ann)))
                            .collect(),
                        required_count: params.iter().filter(|p| !p.variadic && p.default.is_none()).count(),
                        return_type: return_type.as_deref().and_then(InferredType::from_ann),
                        variadic_type: variadic_param.and_then(|p| p.type_ann.as_deref().and_then(InferredType::from_ann)),
                    };
                    tmethods.entry(mname.clone()).or_default().push(sig);
                    self.collect(method_body);
                }
                Stmt::Field { name: fname, kind, type_ann, .. } => {
                    let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                    tfields.insert(fname.clone(), (kind.clone(), ty));
                }
                _ => {}
            }
        }
        self.reg.trait_method_sigs.insert(name.to_string(), tmethods);
        self.reg.trait_field_details.insert(name.to_string(), tfields);
    }

    /// protocol 本体のフィールド・メソッド要件を収集して `known_protocols` に登録する。
    fn collect_protocol(&mut self, name: &str, body: &[Stmt]) {
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        for s in body.iter() {
            match s {
                Stmt::Field { name: fname, kind, type_ann, .. } => {
                    let ty = InferredType::from_ann(type_ann)
                        .unwrap_or(InferredType::Unresolved);
                    fields.push(ProtocolField {
                        name: fname.clone(),
                        kind: kind.clone(),
                        ty,
                    });
                }
                Stmt::FnDef { name: mname, params, return_type, .. } => {
                    let ret = return_type
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    let method_params: Vec<(String, bool, InferredType)> = params
                        .iter()
                        .filter(|p| p.name != "self")
                        .map(|p| {
                            let ty = p
                                .type_ann
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Unresolved);
                            (p.name.clone(), p.mutable, ty)
                        })
                        .collect();
                    methods.push(ProtocolMethod {
                        name: mname.clone(),
                        params: method_params,
                        return_type: ret,
                    });
                }
                _ => {}
            }
        }
        self.reg
            .known_protocols
            .insert(name.to_string(), ProtocolInfo { fields, methods });
    }
}

#![allow(dead_code)]

mod types;
mod errors;
mod scope;
mod stmt;
mod infer;
mod type_utils;
mod call_check;
mod binop;
mod decorator;

#[allow(unused_imports)]
pub use types::{FnTypeParam, InferredType};
#[allow(unused_imports)]
pub use errors::{StaticTypeError, TypeErrorKind};
use types::{FnSig, VarInfo};

use std::collections::{HashMap, HashSet};
use crate::ast::{Accessibility, Stmt};

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

pub struct TypeChecker {
    scope_stack: Vec<HashMap<String, VarInfo>>,
    fn_sigs: HashMap<String, Vec<FnSig>>,
    class_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    known_class_names: HashSet<String>,
    new_type_originals: HashMap<String, String>,
    class_bases: HashMap<String, Vec<String>>,
    class_fields: HashMap<String, HashMap<String, bool>>,
    class_member_access: HashMap<String, HashMap<String, Accessibility>>,
    class_static_methods: HashMap<String, HashSet<String>>,
    current_fn_name: Option<String>,
    current_class_name: Option<String>,
    block_return_forbidden_depth: usize,
    pub errors: Vec<StaticTypeError>,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut global: HashMap<String, VarInfo> = HashMap::new();
        let builtins: &[(&str, InferredType)] = &[
            ("int", InferredType::Int),
            ("float", InferredType::Float),
            ("str", InferredType::Str),
            ("bool", InferredType::Bool),
            ("Any", InferredType::Any),
            (
                "function",
                InferredType::Function {
                    params: None,
                    return_type: Box::new(InferredType::Any),
                },
            ),
        ];
        for (name, inner) in builtins {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(inner.clone())),
                    mutable: false,
                },
            );
        }
        let mut known_class_names: HashSet<String> = HashSet::new();
        let mut new_type_originals: HashMap<String, String> = HashMap::new();
        for (cls_name, prim_type) in [("path", "str"), ("Index", "int"), ("Size", "int")] {
            known_class_names.insert(cls_name.to_string());
            new_type_originals.insert(cls_name.to_string(), prim_type.to_string());
        }
        known_class_names.insert("slice".to_string());
        for name in ["begin", "last"] {
            global.insert(
                name.to_string(),
                VarInfo {
                    ty: InferredType::NamedInstance("Index".to_string()),
                    mutable: false,
                },
            );
        }
        global.insert(
            "Error".to_string(),
            VarInfo {
                ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                    "Error".to_string(),
                ))),
                mutable: false,
            },
        );
        let exception_names = [
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
        for class_name in exception_names {
            known_class_names.insert(class_name.to_string());
            global.insert(
                class_name.to_string(),
                VarInfo {
                    ty: InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                        class_name.to_string(),
                    ))),
                    mutable: false,
                },
            );
        }
        let mut class_bases: HashMap<String, Vec<String>> = HashMap::new();
        for class_name in exception_names {
            class_bases.insert(class_name.to_string(), vec!["Error".to_string()]);
        }
        Self {
            scope_stack: vec![global],
            fn_sigs: HashMap::new(),
            class_method_sigs: HashMap::new(),
            known_class_names,
            new_type_originals,
            class_bases,
            class_fields: HashMap::new(),
            class_member_access: HashMap::new(),
            class_static_methods: HashMap::new(),
            current_fn_name: None,
            current_class_name: None,
            block_return_forbidden_depth: 0,
            errors: Vec::new(),
        }
    }

    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new();
        tc.collect_fn_sigs(stmts);
        tc.check_stmts(stmts);
        tc.errors
    }

    pub(crate) fn collect_fn_sigs(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::FnDef {
                    name,
                    params,
                    return_type,
                    body,
                    ..
                } => {
                    let sig = FnSig {
                        params: params
                            .iter()
                            .map(|p| {
                                (
                                    p.name.clone(),
                                    p.type_ann.as_deref().and_then(InferredType::from_ann),
                                )
                            })
                            .collect(),
                        required_count: params.iter().filter(|p| p.default.is_none()).count(),
                        return_type: return_type.as_deref().and_then(InferredType::from_ann),
                    };
                    self.fn_sigs.entry(name.clone()).or_default().push(sig);
                    self.collect_fn_sigs(body);
                }
                Stmt::ClassDef {
                    name, bases, body, ..
                } => {
                    self.known_class_names.insert(name.clone());
                    self.class_bases.insert(name.clone(), bases.clone());
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
                            let storage_name = if mname == "__cast__" && !template_params.is_empty()
                            {
                                format!("__cast__[{}]", template_params[0].name)
                            } else {
                                mname.clone()
                            };
                            let sig = FnSig {
                                params: params
                                    .iter()
                                    .map(|p| {
                                        (
                                            p.name.clone(),
                                            p.type_ann.as_deref().and_then(InferredType::from_ann),
                                        )
                                    })
                                    .collect(),
                                required_count: params
                                    .iter()
                                    .filter(|p| p.default.is_none())
                                    .count(),
                                return_type: return_type
                                    .as_deref()
                                    .and_then(InferredType::from_ann),
                            };
                            cls_methods.entry(storage_name).or_default().push(sig);
                        }
                    }
                    self.class_method_sigs.insert(name.clone(), cls_methods);
                    let mut fields: HashMap<String, bool> = HashMap::new();
                    let mut member_access: HashMap<String, Accessibility> = HashMap::new();
                    let mut static_methods: HashSet<String> = HashSet::new();
                    for s in body.iter() {
                        match s {
                            Stmt::Field {
                                name: fname,
                                kind,
                                access,
                                ..
                            } => {
                                let mutable = matches!(kind, crate::ast::FieldKind::Mut);
                                fields.insert(fname.clone(), mutable);
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
                    self.class_fields.insert(name.clone(), fields);
                    self.class_member_access.insert(name.clone(), member_access);
                    if !static_methods.is_empty() {
                        self.class_static_methods
                            .insert(name.clone(), static_methods);
                    }
                    self.collect_fn_sigs(body);
                }
                Stmt::EnumDef { name, .. } => {
                    self.known_class_names.insert(name.clone());
                    let item_type_name = format!("enum_item_{}", name);
                    self.known_class_names.insert(item_type_name);
                }
                Stmt::TraitDef { body, .. } => self.collect_fn_sigs(body),
                Stmt::Match { arms, .. } => {
                    for arm in arms {
                        self.collect_fn_sigs(&arm.body);
                    }
                }
                Stmt::If {
                    branches,
                    else_body,
                } => {
                    for (_, body) in branches {
                        self.collect_fn_sigs(body);
                    }
                    if let Some(body) = else_body {
                        self.collect_fn_sigs(body);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    self.collect_fn_sigs(body);
                }
                _ => {}
            }
        }
        for stmt in stmts {
            if let Stmt::NewTypeDef { name, original } = stmt {
                self.known_class_names.insert(name.clone());
                self.new_type_originals
                    .insert(name.clone(), original.clone());
                if let Some(orig_sigs) = self.class_method_sigs.get(original).cloned() {
                    self.class_method_sigs.insert(name.clone(), orig_sigs);
                }
            }
        }
    }
}

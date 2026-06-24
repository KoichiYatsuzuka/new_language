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
pub use errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use types::{FnSig, ProtocolField, ProtocolInfo, ProtocolMethod, VarInfo};

use std::collections::{HashMap, HashSet};
use crate::ast::{Accessibility, FieldKind, Stmt};

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

/// 静的型検査器。AST を走査してすべての型エラーを収集し報告する。
pub struct TypeChecker {
    /// 変数スコープのスタック。インデックス 0 がグローバルスコープ、末尾がローカルスコープ。
    /// 各エントリは変数名 → `VarInfo`（型・可変フラグ）のマップ。
    scope_stack: Vec<HashMap<String, VarInfo>>,
    /// トップレベルおよびネストした関数のシグネチャキャッシュ。
    /// キー: 関数名、値: オーバーロード候補の `FnSig` リスト。
    fn_sigs: HashMap<String, Vec<FnSig>>,
    /// クラスメソッドのシグネチャキャッシュ。
    /// キー: クラス名 → (メソッド名 → `FnSig` リスト)。
    class_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    /// パース済みクラス・new_type 名の集合。`NamedInstance` の解決に使用する。
    known_class_names: HashSet<String>,
    /// `new_type Name: Original` の元の型名マップ。
    /// キー: 新しい型名、値: 元の型名（プリミティブ名またはクラス名）。
    new_type_originals: HashMap<String, String>,
    /// クラスの基底クラス・トレイト名のリスト。
    /// キー: クラス名、値: 基底クラス/トレイト名のリスト。継承チェック・protected アクセス検査に使用。
    class_bases: HashMap<String, Vec<String>>,
    /// クラスフィールドの可変フラグマップ。
    /// キー: クラス名 → (フィールド名 → 可変フラグ)。`let` フィールドへの代入チェックに使用。
    class_fields: HashMap<String, HashMap<String, bool>>,
    /// クラスフィールドの詳細（種別・型）。Protocol 適合チェックで使用する。
    /// キー: クラス名 → (フィールド名 → (FieldKind, InferredType))。
    class_field_details: HashMap<String, HashMap<String, (FieldKind, InferredType)>>,
    /// クラスメンバーのアクセス可能性マップ。
    /// キー: クラス名 → (メンバー名 → `Accessibility`)。`Public` 以外のみ格納。
    class_member_access: HashMap<String, HashMap<String, Accessibility>>,
    /// `static fn` で定義されたスタティックメソッド名の集合。
    /// キー: クラス名、値: スタティックメソッド名のセット。インスタンス経由の呼び出しをエラーとして検出する。
    class_static_methods: HashMap<String, HashSet<String>>,
    /// 現在型検査中の関数名。`None` はトップレベルまたはクラス本体を示す。
    /// メソッドの `Self` 型チェック・`__init__` 内の不変フィールド代入許可に使用する。
    current_fn_name: Option<String>,
    /// 現在型検査中のクラス名。`None` はクラス外を示す。
    /// `private`/`protected` メンバーへのアクセス検査・`Self` 型解決に使用する。
    current_class_name: Option<String>,
    /// `for`/`while`/`match`/`block` 式の入れ子深さ。
    /// `block_return` を直接含む `for`/`while` 式ボディを検出するために使用する。
    /// この値が 1 以上のとき `block_return` は型エラー `BlockReturnInLoopExpr` を発生させる。
    block_return_forbidden_depth: usize,
    /// 収集された静的型エラーのリスト。`check()` が返す前にここへ蓄積される。
    pub errors: Vec<StaticTypeError>,
    /// 収集された静的型警告のリスト。
    pub warnings: Vec<StaticTypeWarning>,
    /// プロトコル定義の情報。プロトコル名 → ProtocolInfo。
    pub(crate) known_protocols: HashMap<String, ProtocolInfo>,
}

impl TypeChecker {
    /// 組み込み型・例外クラスを登録した初期状態の [`TypeChecker`] を生成する。
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
            class_field_details: HashMap::new(),
            class_member_access: HashMap::new(),
            class_static_methods: HashMap::new(),
            current_fn_name: None,
            current_class_name: None,
            block_return_forbidden_depth: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
            known_protocols: HashMap::new(),
        }
    }

    /// 文のスライスを静的型検査して、収集されたすべての [`StaticTypeError`] を返す。
    pub fn check(stmts: &[Stmt]) -> Vec<StaticTypeError> {
        let mut tc = Self::new();
        tc.collect_fn_sigs(stmts);
        tc.check_stmts(stmts);
        tc.errors
    }

    /// 文のスライスを静的型検査して、エラーと警告を両方返す。
    pub fn check_with_warnings(stmts: &[Stmt]) -> (Vec<StaticTypeError>, Vec<StaticTypeWarning>) {
        let mut tc = Self::new();
        tc.collect_fn_sigs(stmts);
        tc.check_stmts(stmts);
        (tc.errors, tc.warnings)
    }

    /// `NamedInstance(name)` がプロトコル名であれば `Protocol(name)` に変換する。
    pub(crate) fn resolve_protocol_type(&self, ty: InferredType) -> InferredType {
        if let InferredType::NamedInstance(ref name) = ty {
            if self.known_protocols.contains_key(name.as_str()) {
                return InferredType::Protocol(name.clone());
            }
        }
        ty
    }

    /// 文のスライスを先行スキャンして関数・クラス・trait のシグネチャ情報をキャッシュする。
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
                    self.class_method_sigs.insert(name.clone(), cls_methods);
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
                    self.class_fields.insert(name.clone(), fields);
                    self.class_field_details.insert(name.clone(), field_details);
                    self.class_member_access.insert(name.clone(), member_access);
                    if !static_methods.is_empty() {
                        self.class_static_methods
                            .insert(name.clone(), static_methods);
                    }
                    // Only recurse into method bodies for nested closures;
                    // class methods themselves must NOT be added to fn_sigs.
                    for s in body.iter() {
                        if let Stmt::FnDef { body: method_body, .. } = s {
                            self.collect_fn_sigs(method_body);
                        }
                    }
                }
                Stmt::EnumDef { name, .. } => {
                    self.known_class_names.insert(name.clone());
                    let item_type_name = format!("enum_item_{}", name);
                    self.known_class_names.insert(item_type_name);
                }
                Stmt::TraitDef { body, .. } => {
                    // Trait methods must NOT be added to fn_sigs; only recurse into bodies.
                    for s in body.iter() {
                        if let Stmt::FnDef { body: method_body, .. } = s {
                            self.collect_fn_sigs(method_body);
                        }
                    }
                }
                Stmt::ProtocolDef { name, body } => {
                    // プロトコルを known_protocols に登録する
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
                    self.known_protocols.insert(name.clone(), ProtocolInfo { fields, methods });
                }
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

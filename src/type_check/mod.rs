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
    /// トレイトメソッドのシグネチャキャッシュ（Intersection 適合チェックで使用）。
    /// キー: トレイト名 → (メソッド名 → `FnSig` リスト)。
    trait_method_sigs: HashMap<String, HashMap<String, Vec<FnSig>>>,
    /// トレイトフィールドの詳細（種別・型）（Intersection 適合チェックで使用）。
    /// キー: トレイト名 → (フィールド名 → (FieldKind, InferredType))。
    trait_field_details: HashMap<String, HashMap<String, (FieldKind, InferredType)>>,
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
            trait_method_sigs: HashMap::new(),
            trait_field_details: HashMap::new(),
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
                Stmt::TraitDef { name, body, .. } => {
                    // Collect trait method sigs and fields for Intersection member checking.
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
                                self.collect_fn_sigs(method_body);
                            }
                            Stmt::Field { name: fname, kind, type_ann, .. } => {
                                let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                                tfields.insert(fname.clone(), (kind.clone(), ty));
                            }
                            _ => {}
                        }
                    }
                    self.trait_method_sigs.insert(name.clone(), tmethods);
                    self.trait_field_details.insert(name.clone(), tfields);
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

// ---------------------------------------------------------------------------
// Intersection type helpers
// ---------------------------------------------------------------------------

/// メンバーを表す内部型。フィールドかメソッドのいずれか。
#[derive(Clone, Debug)]
pub(crate) enum MemberKind {
    Field {
        kind: FieldKind,
        ty: InferredType,
        access: Accessibility,
    },
    Method {
        sigs: Vec<FnSig>,
        access: Accessibility,
    },
}

impl TypeChecker {
    /// 型から公開メンバーの名前→MemberKind マップを収集する。
    pub(crate) fn get_type_members(&self, ty: &InferredType) -> HashMap<String, MemberKind> {
        let mut members: HashMap<String, MemberKind> = HashMap::new();
        match ty {
            InferredType::NamedInstance(name) => {
                // Class fields
                if let Some(fields) = self.class_field_details.get(name.as_str()) {
                    for (fname, (kind, fty)) in fields {
                        let access = self.class_member_access
                            .get(name.as_str())
                            .and_then(|m| m.get(fname))
                            .cloned()
                            .unwrap_or(Accessibility::Public);
                        members.insert(fname.clone(), MemberKind::Field {
                            kind: kind.clone(),
                            ty: fty.clone(),
                            access,
                        });
                    }
                }
                // Class methods
                if let Some(methods) = self.class_method_sigs.get(name.as_str()) {
                    for (mname, sigs) in methods {
                        let access = self.class_member_access
                            .get(name.as_str())
                            .and_then(|m| m.get(mname))
                            .cloned()
                            .unwrap_or(Accessibility::Public);
                        members.insert(mname.clone(), MemberKind::Method {
                            sigs: sigs.clone(),
                            access,
                        });
                    }
                }
                // Trait fields
                if let Some(fields) = self.trait_field_details.get(name.as_str()) {
                    for (fname, (kind, fty)) in fields {
                        members.entry(fname.clone()).or_insert(MemberKind::Field {
                            kind: kind.clone(),
                            ty: fty.clone(),
                            access: Accessibility::Public,
                        });
                    }
                }
                // Trait methods
                if let Some(methods) = self.trait_method_sigs.get(name.as_str()) {
                    for (mname, sigs) in methods {
                        members.entry(mname.clone()).or_insert(MemberKind::Method {
                            sigs: sigs.clone(),
                            access: Accessibility::Public,
                        });
                    }
                }
            }
            InferredType::Protocol(name) => {
                if let Some(proto) = self.known_protocols.get(name.as_str()) {
                    for f in &proto.fields {
                        members.insert(f.name.clone(), MemberKind::Field {
                            kind: f.kind.clone(),
                            ty: f.ty.clone(),
                            access: Accessibility::Public,
                        });
                    }
                    for m in &proto.methods {
                        let sig = FnSig {
                            params: m.params.iter()
                                .map(|(pname, _pmut, pty)| (pname.clone(), Some(pty.clone())))
                                .collect(),
                            required_count: m.params.len(),
                            return_type: Some(m.return_type.clone()),
                            variadic_type: None,
                        };
                        members.insert(m.name.clone(), MemberKind::Method {
                            sigs: vec![sig],
                            access: Accessibility::Public,
                        });
                    }
                }
            }
            _ => {}
        }
        members
    }

    /// 交差型の構成型間のメンバー互換性を検査し、重複は警告、競合はエラーを収集する。
    pub(crate) fn check_intersection_members(
        &mut self,
        types: &[InferredType],
        span: Option<crate::token::Span>,
    ) {
        use errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};

        if types.len() < 2 {
            return;
        }
        // Collect member maps for each constituent type
        let member_maps: Vec<(String, HashMap<String, MemberKind>)> = types.iter()
            .map(|t| (t.to_string(), self.get_type_members(t)))
            .collect();

        // For each unique member name present in more than one type, compare
        let all_names: HashSet<&String> = member_maps.iter()
            .flat_map(|(_, m)| m.keys())
            .collect();

        for name in all_names {
            // Skip self / cls
            if name == "self" || name == "cls" || name == "__init__" {
                continue;
            }
            let entries: Vec<(&String, &MemberKind)> = member_maps.iter()
                .filter_map(|(tname, m)| m.get(name).map(|mk| (tname, mk)))
                .collect();
            if entries.len() < 2 {
                continue;
            }
            // Compare first entry against subsequent ones
            let (type_a, mk_a) = entries[0];
            for (type_b, mk_b) in &entries[1..] {
                match (mk_a, mk_b) {
                    (MemberKind::Field { kind: ka, ty: ta, access: acc_a },
                     MemberKind::Field { kind: kb, ty: tb, access: acc_b }) => {
                        let type_mismatch = ta != tb;
                        let access_mismatch = acc_a != acc_b;
                        let kind_mismatch = ka != kb;
                        if type_mismatch || access_mismatch || kind_mismatch {
                            let reason = if type_mismatch {
                                format!("field type differs: {} vs {}", ta, tb)
                            } else if access_mismatch {
                                format!("access modifier differs: {:?} vs {:?}", acc_a, acc_b)
                            } else {
                                format!("field qualifier differs: {:?} vs {:?}", ka, kb)
                            };
                            self.errors.push(StaticTypeError {
                                kind: TypeErrorKind::IntersectionMemberConflict {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                    reason,
                                },
                                span: span.clone(),
                            });
                        } else {
                            self.warnings.push(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        }
                    }
                    (MemberKind::Method { sigs: sigs_a, .. },
                     MemberKind::Method { sigs: sigs_b, .. }) => {
                        // Check if any sig from A is identical to any sig from B (same types)
                        // or if there's a non-overloadable conflict
                        let mut found_conflict = false;
                        let mut found_duplicate = false;
                        'outer: for sig_a in sigs_a {
                            for sig_b in sigs_b {
                                // Compare params (ignoring self)
                                let a_params: Vec<_> = sig_a.params.iter().filter(|(n, _)| n != "self").collect();
                                let b_params: Vec<_> = sig_b.params.iter().filter(|(n, _)| n != "self").collect();
                                if a_params.len() != b_params.len() {
                                    // Different param count → overloadable → warning only (handled below)
                                    continue;
                                }
                                // Same param count: check types
                                let same_types = a_params.iter().zip(b_params.iter())
                                    .all(|((_, ta), (_, tb))| ta == tb);
                                if !same_types {
                                    // Different param types → overloadable → warning only
                                    continue;
                                }
                                // Same param count and types: check if names/mutability differ
                                let same_names = a_params.iter().zip(b_params.iter())
                                    .all(|((na, _), (nb, _))| na == nb);
                                if !same_names {
                                    found_conflict = true;
                                    break 'outer;
                                }
                                // Identical → duplicate warning
                                if sig_a.return_type == sig_b.return_type {
                                    found_duplicate = true;
                                }
                            }
                        }
                        if found_conflict {
                            self.errors.push(StaticTypeError {
                                kind: TypeErrorKind::IntersectionMemberConflict {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                    reason: "method signatures have same parameter types but different parameter names (non-overloadable)".to_string(),
                                },
                                span: span.clone(),
                            });
                        } else if found_duplicate {
                            self.warnings.push(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        } else {
                            // Different signatures (overloadable) → warning
                            self.warnings.push(StaticTypeWarning {
                                kind: TypeWarningKind::IntersectionMemberDuplicate {
                                    member_name: name.clone(),
                                    type_a: type_a.clone(),
                                    type_b: type_b.to_string(),
                                },
                                span: span.clone(),
                            });
                        }
                    }
                    _ => {
                        // One is a field, the other is a method — conflict
                        self.errors.push(StaticTypeError {
                            kind: TypeErrorKind::IntersectionMemberConflict {
                                member_name: name.clone(),
                                type_a: type_a.clone(),
                                type_b: type_b.to_string(),
                                reason: "member is a field in one type and a method in another".to_string(),
                            },
                            span: span.clone(),
                        });
                    }
                }
            }
        }
    }

    /// 型 `guard_type` が交差型 `intersection_types` のすべての構成型を満たすか検査する。
    /// 満たさない場合はエラーを収集して `false` を返す。
    pub(crate) fn check_intersection_guard_type(
        &mut self,
        guard_type: &str,
        intersection_types: &[InferredType],
        span: Option<crate::token::Span>,
    ) -> bool {
        use errors::{StaticTypeError, TypeErrorKind};
        let mut ok = true;
        for ty in intersection_types {
            let ty_name = match ty {
                InferredType::NamedInstance(n) | InferredType::Protocol(n) => n.clone(),
                _ => continue,
            };
            let satisfied = if self.known_protocols.contains_key(ty_name.as_str()) {
                // Protocol: check conformance via class field/method existence
                self.class_implements_protocol(guard_type, &ty_name)
            } else {
                // Trait or class: check inheritance
                self.class_implements_trait(guard_type, &ty_name) || guard_type == ty_name
            };
            if !satisfied {
                let intersection_str = format!("Intersection[{}]",
                    intersection_types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", "));
                self.errors.push(StaticTypeError {
                    kind: TypeErrorKind::IntersectionGuardTypeFails {
                        guard_type: guard_type.to_string(),
                        intersection_type: intersection_str,
                        reason: format!("'{}' does not satisfy constraint '{}'", guard_type, ty_name),
                    },
                    span: span.clone(),
                });
                ok = false;
            }
        }
        ok
    }

    /// クラスがプロトコルを満たすかを簡易チェックする（フィールド名・メソッド名の存在確認）。
    fn class_implements_protocol(&self, class_name: &str, protocol_name: &str) -> bool {
        let Some(proto) = self.known_protocols.get(protocol_name) else {
            return false;
        };
        let class_fields = self.class_field_details.get(class_name);
        let class_methods = self.class_method_sigs.get(class_name);
        for f in &proto.fields {
            if class_fields.map_or(true, |m| !m.contains_key(&f.name)) {
                return false;
            }
        }
        for m in &proto.methods {
            if class_methods.map_or(true, |ms| !ms.contains_key(&m.name)) {
                return false;
            }
        }
        true
    }
}

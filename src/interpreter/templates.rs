// templates.rs — テンプレート展開・AST置換
// (check_template_constraints / type_satisfies_trait / instantiate_template / instantiate_template_class)
// + subst_* フリー関数 (AST substitution helpers for template instantiation)
//
// テンプレート関数・クラス・ジェネレータ関数の呼び出し時に型変数を具体型に置換して実行する。
// `subst_*` フリー関数群が AST ノードを再帰的に走査して型変数名を書き換える。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{
    CallArg, ExceptHandler, Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt, TemplateParam,
};

use super::{
    ClassValue, DictData, FnValue, GeneratorFnValue, Interpreter, TemplateClassValue, Value,
};

impl Interpreter {
    /// 各具体型引数がテンプレートパラメータの trait 制約を満たすか検証する。
    ///
    /// - `template_params`: テンプレートパラメータリスト（型変数名と制約）
    /// - `type_args`: 呼び出し時に渡された具体型名のリスト
    ///
    /// 戻り値: `Ok(())` — すべての制約を満たす。`Err(message)` — 型引数数不一致または制約違反
    pub(super) fn check_template_constraints(
        &self,
        template_params: &[TemplateParam],
        type_args: &[String],
    ) -> Result<(), String> {
        if template_params.len() != type_args.len() {
            return Err(format!(
                "TemplateError: expected {} type argument(s), got {}",
                template_params.len(),
                type_args.len()
            ));
        }
        // 各型変数とその具体型を対応付けて制約を検証する
        for (param, type_name) in template_params.iter().zip(type_args.iter()) {
            for constraint in &param.constraints {
                if !self.type_satisfies_trait(type_name, constraint)? {
                    return Err(format!(
                        "TemplateError: type `{type_name}` does not satisfy trait `{constraint}` \
                         (required for template parameter `{}`)",
                        param.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// 指定した型名が trait を実装しているか（`bases` に含まれているか）を返す。
    ///
    /// 組み込み型（`int`, `str` 等）は trait を実装していないため常に `false` を返す。
    ///
    /// - `type_name`: 検査する型の名前（スコープから検索される）
    /// - `trait_name`: 実装されているか確認する trait 名
    ///
    /// 戻り値: `Ok(true)` — 実装あり、`Ok(false)` — 実装なし、`Err` — 型が未定義
    pub(super) fn type_satisfies_trait(
        &self,
        type_name: &str,
        trait_name: &str,
    ) -> Result<bool, String> {
        match self.get_val(type_name) {
            Some(Value::Class(cls)) => Ok(cls.bases.contains(&trait_name.to_string())),
            Some(_) => Ok(false), // 組み込み型や非クラス値は trait を実装していない
            None => Err(format!("NameError: type `{type_name}` is not defined")),
        }
    }

    /// テンプレート関数・クラス・ジェネレータを型引数で実体化して実行または構築する。
    ///
    /// ディスパッチ先:
    /// - `TemplateFn`: 型変数を置換して通常関数として実行
    /// - `TemplateClass`: 型変数を置換してクラスを構築してインスタンス化
    /// - `TemplateGenFn`: 型変数を置換してジェネレータ関数として実行
    /// - `Type("dict")`: `dict[K, V](...)` の組み込み辞書コンストラクタとして処理
    ///
    /// - `tmpl_val`: 実体化するテンプレート値
    /// - `type_args`: 具体型名のリスト（テンプレートパラメータと同数）
    /// - `call_args`: 実体化後の関数/コンストラクタ呼び出し引数
    ///
    /// 戻り値: `Ok(Value)` — 実行結果またはインスタンス。`Err(message)` — 制約違反・型エラー等
    pub(super) fn instantiate_template(
        &mut self,
        tmpl_val: Value,
        type_args: &[String],
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        match tmpl_val {
            Value::TemplateFn(tmpl) => {
                // テンプレート関数: 制約を検証し、型変数を具体型に置換して通常関数として実行する。
                // 制約検証は毎回行う（安価・エラー意味論を保つ）。AST 置換と FnValue 構築は
                // `(テンプレート, 型引数)` でメモ化し、再実体化で clone-walk と Chunk 再コンパイルを省く（#7）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                let fn_val = match self.template_fn_cache.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let type_map: HashMap<String, String> = tmpl
                            .template_params
                            .iter()
                            .zip(type_args.iter())
                            .map(|(p, t)| (p.name.clone(), t.clone()))
                            .collect();
                        let concrete_params = subst_params(&tmpl.params, &type_map);
                        let concrete_body = subst_stmts(&tmpl.body, &type_map);
                        let fn_val = Rc::new(FnValue {
                            name: tmpl.name.clone(),
                            params: concrete_params,
                            body: concrete_body,
                            is_python: false,
                            captured_env: std::collections::HashMap::new(),
                            return_type: None,
                        });
                        self.template_fn_cache.insert(key, fn_val.clone());
                        fn_val
                    }
                };
                self.exec_fn(fn_val, call_args, None, "<template_fn>", None)
            }
            Value::TemplateClass(tmpl) => {
                // テンプレートクラス: 制約を検証し、型変数を置換してクラスを構築・インスタンス化する
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let type_map: HashMap<String, String> = tmpl
                    .template_params
                    .iter()
                    .zip(type_args.iter())
                    .map(|(p, t)| (p.name.clone(), t.clone()))
                    .collect();
                let concrete_body = subst_stmts(&tmpl.body, &type_map);
                self.instantiate_template_class(&tmpl, concrete_body, call_args)
            }
            Value::TemplateGenFn(tmpl) => {
                // テンプレートジェネレータ関数: 型変数を置換してジェネレータとして実行する。
                // TemplateFn と同様に `(テンプレート, 型引数)` でメモ化（#7）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                let gen_fn = match self.template_gen_cache.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let type_map: HashMap<String, String> = tmpl
                            .template_params
                            .iter()
                            .zip(type_args.iter())
                            .map(|(p, t)| (p.name.clone(), t.clone()))
                            .collect();
                        let concrete_params = subst_params(&tmpl.params, &type_map);
                        let concrete_body = subst_stmts(&tmpl.body, &type_map);
                        let gen_fn = Rc::new(GeneratorFnValue {
                            name: tmpl.name.clone(),
                            params: concrete_params,
                            body: concrete_body,
                            captured_env: std::collections::HashMap::new(),
                        });
                        self.template_gen_cache.insert(key, gen_fn.clone());
                        gen_fn
                    }
                };
                self.exec_generator(gen_fn, call_args, None)
            }
            // Signal[T]() — 型付きシグナルを生成する（型引数は型チェックの注釈としてのみ使用）
            Value::Type(ref t) if t == "Signal" => {
                Ok(Value::Signal(std::rc::Rc::new(std::cell::RefCell::new(
                    super::event_loop::SignalData::new(),
                ))))
            }
            // 組み込み辞書型コンストラクタ: `dict[KeyType, ItemType](...)`
            Value::Type(ref t) if t == "dict" => {
                if type_args.len() != 2 {
                    return Err(format!(
                        "TypeError: dict requires exactly 2 type arguments [key_type, item_type], got {}",
                        type_args.len()
                    ));
                }
                let key_type = type_args[0].clone();
                let item_type = type_args[1].clone();

                if call_args.is_empty() {
                    // `dict[K, V]()` — 空の型付き辞書を生成する
                    Ok(Value::Dict(Rc::new(RefCell::new(DictData::new(
                        key_type, item_type,
                    )))))
                } else if call_args.len() == 1 {
                    // `dict[K, V]({key: val, ...})` — 辞書リテラルから型付き辞書を生成する
                    let arg_val = self.eval(call_args[0].expr())?;
                    match arg_val {
                        Value::Dict(src_rc) => {
                            let src = src_rc.borrow();
                            let src_keys = src.all_keys();
                            let src_vals = src.all_items();
                            // 各キーと値が宣言された型と一致するか検査する
                            for k in &src_keys {
                                if !Self::value_matches_type(k, &key_type) {
                                    return Err(format!(
                                        "StaticTypeError: dict key type mismatch: \
                                         expected '{}', got '{}'",
                                        key_type,
                                        self.type_name(k)
                                    ));
                                }
                            }
                            for v in &src_vals {
                                if !Self::value_matches_type(v, &item_type) {
                                    return Err(format!(
                                        "StaticTypeError: dict item type mismatch: \
                                         expected '{}', got '{}'",
                                        item_type,
                                        self.type_name(v)
                                    ));
                                }
                            }
                            // 型チェック通過後にソースデータをコピーして新しい型付き辞書を構築する
                            let mut new_data = DictData::new(key_type, item_type);
                            for (k, v) in src_keys.into_iter().zip(src_vals) {
                                new_data.set(k, v);
                            }
                            Ok(Value::Dict(Rc::new(RefCell::new(new_data))))
                        }
                        _ => Err(
                            "TypeError: dict constructor argument must be a dict literal `{...}`"
                                .to_string(),
                        ),
                    }
                } else {
                    Err("TypeError: dict constructor takes 0 or 1 argument".to_string())
                }
            }
            _ => Err("TemplateError: expression is not a template".to_string()),
        }
    }

    /// 型変数が置換されたテンプレートクラス本体から具体的な `ClassValue` を構築してインスタンス化する。
    ///
    /// `exec` の `Stmt::ClassDef` 処理と同様にクラス本体を走査してメソッド・フィールド・クラス変数を収集し、
    /// `ClassValue` を構築してから `instantiate` でインスタンスを生成する。
    ///
    /// - `tmpl`: 元のテンプレートクラス定義（名前・bases を参照する）
    /// - `concrete_body`: 型変数が具体型に置換済みのクラス本体文リスト
    /// - `call_args`: コンストラクタ呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Value::Instance)` — 構築済みインスタンス。`Err` — 実行エラー
    pub(super) fn instantiate_template_class(
        &mut self,
        tmpl: &TemplateClassValue,
        concrete_body: Vec<Stmt>,
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
        let mut field_defaults = Vec::new();
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        let mut field_access: HashMap<String, crate::ast::Accessibility> = HashMap::new();
        let mut method_access: HashMap<String, crate::ast::Accessibility> = HashMap::new();
        let mut static_method_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut class_method_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut static_vars: HashMap<String, Rc<RefCell<Value>>> = HashMap::new();
        let mut own_field_order: Vec<(String, bool)> = Vec::new();
        for stmt in &concrete_body {
            match stmt {
                Stmt::FnDef {
                    name: mname,
                    template_params,
                    params,
                    body: mbody,
                    access: macc,
                    is_static,
                    is_class_method,
                    ..
                } => {
                    let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                        format!("__cast__[{}]", template_params[0].name)
                    } else {
                        mname.clone()
                    };
                    if *is_static {
                        static_method_names.insert(storage_name.clone());
                    }
                    if *is_class_method {
                        class_method_names.insert(storage_name.clone());
                    }
                    if *macc != crate::ast::Accessibility::Public {
                        method_access.insert(storage_name.clone(), macc.clone());
                    }
                    methods
                        .entry(storage_name)
                        .or_default()
                        .push(Rc::new(FnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            is_python: false,
                            captured_env: std::collections::HashMap::new(),
                        return_type: None,
                        }));
                }
                Stmt::GenDef {
                    name: mname,
                    params,
                    body: mbody,
                    access: macc,
                    ..
                } => {
                    if *macc != crate::ast::Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    gen_methods.insert(
                        mname.clone(),
                        Rc::new(GeneratorFnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            captured_env: std::collections::HashMap::new(),
                        }),
                    );
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::Const,
                    default: Some(init),
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = self.eval(init)?;
                    class_vars.insert(fname.clone(), val);
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::StaticMut,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = if let Some(init) = default {
                        self.eval(init)?
                    } else {
                        Value::None
                    };
                    static_vars.insert(fname.clone(), Rc::new(RefCell::new(val)));
                }
                Stmt::Field {
                    name: fname,
                    kind,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let mutable = *kind == FieldKind::Mut;
                    field_mutability.insert(fname.clone(), mutable);
                    own_field_order.push((fname.clone(), mutable));
                    if let Some(init) = default {
                        let val = self.eval(init)?;
                        field_defaults.push((fname.clone(), val, mutable));
                    }
                }
                _ => {}
            }
        }
        let (field_index, field_mutability_vec, field_count) =
            self.build_field_index(&own_field_order, &tmpl.bases);
        let cls = Rc::new(ClassValue {
            name: tmpl.name.clone(),
            class_id: crate::interpreter::value::alloc_class_id(),
            bases: tmpl.bases.clone(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
            field_index,
            field_count,
            field_mutability_vec,
            field_access,
            method_access,
            static_method_names,
            class_method_names,
            static_vars,
            new_type_base: None,
            is_exception: false,
            raw_layout: None,
        });
        self.instantiate(cls, call_args)
    }
}

// ---------------------------------------------------------------------------
// AST 置換ヘルパー（テンプレート実体化用）
// ---------------------------------------------------------------------------
// `type_map` は型変数名 → 具体型名のマップ。
// `subst_*` 関数群は AST ノードを再帰的に走査し、型変数名を具体型名に書き換えた新しい AST を返す。
// コードのロジック自体は変更せず、型アノテーション部分のみを置換する。

/// 型名文字列を置換する。`type_map` にある型変数名なら具体型名に、なければそのまま返す。
fn subst_type(type_name: &str, type_map: &HashMap<String, String>) -> String {
    type_map
        .get(type_name)
        .cloned()
        .unwrap_or_else(|| type_name.to_string())
}

/// 仮引数リストの型アノテーションを置換した新しいリストを返す。
fn subst_params(params: &[Param], type_map: &HashMap<String, String>) -> Vec<Param> {
    params
        .iter()
        .map(|p| Param {
            name: p.name.clone(),
            mutable: p.mutable,
            type_ann: p.type_ann.as_ref().map(|t| subst_type(t, type_map)),
            default: p.default.clone(),
            variadic: p.variadic,
        })
        .collect()
}

/// 呼び出し引数の式部分を置換した新しい `CallArg` を返す。
fn subst_call_arg(arg: &CallArg, type_map: &HashMap<String, String>) -> CallArg {
    match arg {
        CallArg::Positional(e) => CallArg::Positional(subst_expr(e, type_map)),
        CallArg::Keyword { name, value } => CallArg::Keyword {
            name: name.clone(),
            value: subst_expr(value, type_map),
        },
        CallArg::Variadic(exprs) => {
            CallArg::Variadic(exprs.iter().map(|e| subst_expr(e, type_map)).collect())
        }
    }
}

/// 式内の型変数名を具体型名に置換した新しい `Expr` を返す。
/// リテラル・識別子などは変更せず、再帰的にサブ式を置換する。
fn subst_expr(expr: &Expr, type_map: &HashMap<String, String>) -> Expr {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::ImaginaryLit(_)
        | Expr::Str(_) | Expr::Bool(_) | Expr::None | Expr::Undefined => expr.clone(),
        Expr::Ident(name) => Expr::Ident(name.clone()),
        // テンプレート関数はリゾルバの対象外なので LocalRef は現れないが、網羅性のため保持する。
        Expr::LocalRef { name, slot } => Expr::LocalRef { name: name.clone(), slot: *slot },
        Expr::List(items) => Expr::List(items.iter().map(|e| subst_expr(e, type_map)).collect()),
        Expr::Attr { object, attr, span, .. } => Expr::Attr {
            object: Box::new(subst_expr(object, type_map)),
            attr: attr.clone(),
            span: span.clone(),
            cache: Default::default(),
        },
        Expr::TraitAccess {
            object,
            trait_name,
            attr,
        } => Expr::TraitAccess {
            object: Box::new(subst_expr(object, type_map)),
            trait_name: trait_name.clone(),
            attr: attr.clone(),
        },
        Expr::BinOp {
            op,
            left,
            right,
            span,
        } => Expr::BinOp {
            op: op.clone(),
            left: Box::new(subst_expr(left, type_map)),
            right: Box::new(subst_expr(right, type_map)),
            span: span.clone(),
        },
        Expr::UnaryOp { op, operand } => Expr::UnaryOp {
            op: op.clone(),
            operand: Box::new(subst_expr(operand, type_map)),
        },
        Expr::Call { func, args, span, .. } => Expr::Call {
            func: Box::new(subst_expr(func, type_map)),
            args: args.iter().map(|a| subst_call_arg(a, type_map)).collect(),
            span: span.clone(),
            cache: Default::default(),
        },
        Expr::TemplateInstantiate { base, type_args } => Expr::TemplateInstantiate {
            base: Box::new(subst_expr(base, type_map)),
            type_args: type_args.iter().map(|t| subst_type(t, type_map)).collect(),
        },
        Expr::Subscript { object, index } => Expr::Subscript {
            object: Box::new(subst_expr(object, type_map)),
            index: Box::new(subst_expr(index, type_map)),
        },
        Expr::Slice { begin, end, step } => Expr::Slice {
            begin: begin.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
            end: end.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
            step: step.as_ref().map(|e| Box::new(subst_expr(e, type_map))),
        },
        Expr::Dict(pairs) => Expr::Dict(
            pairs
                .iter()
                .map(|(k, v)| (subst_expr(k, type_map), subst_expr(v, type_map)))
                .collect(),
        ),
        Expr::Tuple(items) => Expr::Tuple(items.iter().map(|e| subst_expr(e, type_map)).collect()),
        Expr::IsType {
            expr,
            negated,
            type_name,
            span,
        } => Expr::IsType {
            expr: Box::new(subst_expr(expr, type_map)),
            negated: *negated,
            type_name: subst_type(type_name, type_map),
            span: span.clone(),
        },
        Expr::Block { stmts, return_type } => Expr::Block {
            stmts: subst_stmts(stmts, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::IfExpr {
            branches,
            else_body,
            return_type,
        } => Expr::IfExpr {
            branches: branches
                .iter()
                .map(|(c, b)| (subst_expr(c, type_map), subst_stmts(b, type_map)))
                .collect(),
            else_body: else_body.as_ref().map(|b| subst_stmts(b, type_map)),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::ForExpr {
            target,
            iter,
            body,
            return_type,
        } => Expr::ForExpr {
            target: target.clone(),
            iter: Box::new(subst_expr(iter, type_map)),
            body: subst_stmts(body, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::WhileExpr {
            cond,
            body,
            return_type,
        } => Expr::WhileExpr {
            cond: Box::new(subst_expr(cond, type_map)),
            body: subst_stmts(body, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::MatchExpr {
            subject,
            arms,
            return_type,
        } => Expr::MatchExpr {
            subject: Box::new(subst_expr(subject, type_map)),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern: match &arm.pattern {
                        MatchPattern::Case(e) => MatchPattern::Case(subst_expr(e, type_map)),
                        MatchPattern::IsType(t) => MatchPattern::IsType(subst_type(t, type_map)),
                    },
                    body: subst_stmts(&arm.body, type_map),
                })
                .collect(),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
        },
        Expr::Set(items) => Expr::Set(items.iter().map(|e| subst_expr(e, type_map)).collect()),
        Expr::Cast {
            object,
            type_name,
            span,
        } => Expr::Cast {
            object: Box::new(subst_expr(object, type_map)),
            type_name: subst_type(type_name, type_map),
            span: span.clone(),
        },
        Expr::DebugVar(name) => Expr::DebugVar(name.clone()),
        Expr::LocalVar(name) => Expr::LocalVar(name.clone()),
        Expr::MustBe { expr, guard_type, span, node_id } => Expr::MustBe {
            expr: Box::new(subst_expr(expr, type_map)),
            guard_type: guard_type.clone(),
            span: span.clone(),
            // テンプレ実体化のクローン: node_id を引き継ぐ（テンプレ対応は #16 次段）。
            node_id: *node_id,
        },
    }
}

/// 文リスト全体を再帰的に置換した新しいリストを返す。
fn subst_stmts(stmts: &[Stmt], type_map: &HashMap<String, String>) -> Vec<Stmt> {
    stmts.iter().map(|s| subst_stmt(s, type_map)).collect()
}

/// 文内の型変数名を具体型名に置換した新しい `Stmt` を返す。
/// 各バリアントを再帰的に処理し、型アノテーション・式・サブ文をすべて置換する。
fn subst_stmt(stmt: &Stmt, type_map: &HashMap<String, String>) -> Stmt {
    match stmt {
        Stmt::Expr(e) => Stmt::Expr(subst_expr(e, type_map)),
        Stmt::Let(name, ann, e) => Stmt::Let(name.clone(), ann.clone(), subst_expr(e, type_map)),
        Stmt::Const(name, ann, e) => Stmt::Const(name.clone(), ann.clone(), subst_expr(e, type_map)),
        Stmt::Mut(name, ann, e) => Stmt::Mut(name.clone(), ann.clone(), subst_expr(e, type_map)),
        Stmt::LetTuple {
            targets,
            value,
            span,
        } => Stmt::LetTuple {
            targets: targets.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
        },
        Stmt::Assign { name, value, span, .. } => Stmt::Assign {
            name: name.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
            slot: Default::default(),
        },
        Stmt::AttrAssign { target, value } => Stmt::AttrAssign {
            target: subst_expr(target, type_map),
            value: subst_expr(value, type_map),
        },
        Stmt::AttrCompoundAssign { target, op, value } => Stmt::AttrCompoundAssign {
            target: subst_expr(target, type_map),
            op: op.clone(),
            value: subst_expr(value, type_map),
        },
        Stmt::CompoundAssign {
            name,
            op,
            value,
            span,
            ..
        } => Stmt::CompoundAssign {
            name: name.clone(),
            op: op.clone(),
            value: subst_expr(value, type_map),
            span: span.clone(),
            slot: Default::default(),
        },
        Stmt::If {
            branches,
            else_body,
        } => Stmt::If {
            branches: branches
                .iter()
                .map(|(cond, body)| (subst_expr(cond, type_map), subst_stmts(body, type_map)))
                .collect(),
            else_body: else_body.as_ref().map(|b| subst_stmts(b, type_map)),
        },
        Stmt::While { cond, body } => Stmt::While {
            cond: subst_expr(cond, type_map),
            body: subst_stmts(body, type_map),
        },
        Stmt::For {
            targets,
            iter,
            body,
        } => Stmt::For {
            targets: targets.clone(),
            iter: subst_expr(iter, type_map),
            body: subst_stmts(body, type_map),
        },
        Stmt::Block(body) => Stmt::Block(subst_stmts(body, type_map)),
        Stmt::Return(e) => Stmt::Return(e.as_ref().map(|e| subst_expr(e, type_map))),
        Stmt::Break => Stmt::Break,
        Stmt::Continue => Stmt::Continue,
        Stmt::Pass => Stmt::Pass,
        Stmt::BlockReturn(e, span) => Stmt::BlockReturn(subst_expr(e, type_map), span.clone()),
        Stmt::LoopYield(e) => Stmt::LoopYield(subst_expr(e, type_map)),
        Stmt::Yield(e) => Stmt::Yield(subst_expr(e, type_map)),
        Stmt::GenDef {
            name,
            template_params,
            params,
            yield_type,
            body,
            access,
        } => Stmt::GenDef {
            name: name.clone(),
            template_params: template_params.clone(),
            params: params.clone(),
            yield_type: yield_type.clone(),
            body: subst_stmts(body, type_map),
            access: access.clone(),
        },
        Stmt::FnDef {
            name,
            template_params,
            params,
            return_type,
            body,
            is_abstract,
            is_static,
            is_class_method,
            decorators,
            access,
        } => Stmt::FnDef {
            name: name.clone(),
            template_params: template_params.clone(),
            params: subst_params(params, type_map),
            return_type: return_type.as_ref().map(|t| subst_type(t, type_map)),
            body: subst_stmts(body, type_map),
            is_abstract: *is_abstract,
            is_static: *is_static,
            is_class_method: *is_class_method,
            decorators: decorators.clone(),
            access: access.clone(),
        },
        Stmt::ClassDef {
            name,
            template_params,
            bases,
            body,
            decorators,
        } => Stmt::ClassDef {
            name: name.clone(),
            template_params: template_params.clone(),
            bases: bases.clone(),
            body: subst_stmts(body, type_map),
            decorators: decorators.clone(),
        },
        Stmt::TraitDef {
            name,
            template_params,
            body,
        } => Stmt::TraitDef {
            name: name.clone(),
            template_params: template_params.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::ProtocolDef { name, body } => Stmt::ProtocolDef {
            name: name.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::Field {
            name,
            kind,
            type_ann,
            default,
            access,
        } => Stmt::Field {
            name: name.clone(),
            kind: kind.clone(),
            type_ann: subst_type(type_ann, type_map),
            default: default.as_ref().map(|e| subst_expr(e, type_map)),
            access: access.clone(),
        },
        Stmt::Freeze(name, span) => Stmt::Freeze(name.clone(), span.clone()),
        Stmt::Static(name, e, span) => {
            Stmt::Static(name.clone(), subst_expr(e, type_map), span.clone())
        }
        Stmt::NewTypeDef { name, original } => Stmt::NewTypeDef {
            name: name.clone(),
            original: subst_type(original, type_map),
        },
        Stmt::EnumDef { name, variants } => Stmt::EnumDef {
            name: name.clone(),
            variants: variants
                .iter()
                .map(|(vname, vexpr)| {
                    (
                        vname.clone(),
                        vexpr.as_ref().map(|e| subst_expr(e, type_map)),
                    )
                })
                .collect(),
        },
        Stmt::Try {
            body,
            handlers,
            finally_body,
        } => Stmt::Try {
            body: subst_stmts(body, type_map),
            handlers: handlers
                .iter()
                .map(|h| ExceptHandler {
                    exc_type: h.exc_type.clone(),
                    name: h.name.clone(),
                    body: subst_stmts(&h.body, type_map),
                })
                .collect(),
            finally_body: finally_body.as_ref().map(|b| subst_stmts(b, type_map)),
        },
        Stmt::Raise { exc, span } => Stmt::Raise {
            exc: exc.as_ref().map(|e| subst_expr(e, type_map)),
            span: span.clone(),
        },
        // Import 文は型変数置換の対象外（body はパース時に解決済み）
        Stmt::Import {
            lang,
            module,
            with_file,
            alias,
            body,
        } => Stmt::Import {
            lang: lang.clone(),
            module: module.clone(),
            with_file: with_file.clone(),
            alias: alias.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::FromImport {
            lang,
            module,
            with_file,
            names,
            body,
        } => Stmt::FromImport {
            lang: lang.clone(),
            module: module.clone(),
            with_file: with_file.clone(),
            names: names.clone(),
            body: subst_stmts(body, type_map),
        },
        Stmt::Match {
            subject,
            arms,
            span,
        } => Stmt::Match {
            subject: subst_expr(subject, type_map),
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern: match &arm.pattern {
                        MatchPattern::Case(e) => MatchPattern::Case(subst_expr(e, type_map)),
                        MatchPattern::IsType(t) => MatchPattern::IsType(subst_type(t, type_map)),
                    },
                    body: subst_stmts(&arm.body, type_map),
                })
                .collect(),
            span: span.clone(),
        },
        Stmt::AsyncAssign {
            target,
            return_type,
            stmts,
        } => Stmt::AsyncAssign {
            target: target.clone(),
            return_type: return_type.clone(),
            stmts: subst_stmts(stmts, type_map),
        },
        Stmt::BreakPoint { span } => Stmt::BreakPoint { span: span.clone() },
        Stmt::DebugLet(name, e) => Stmt::DebugLet(name.clone(), subst_expr(e, type_map)),
        Stmt::EventSubscribe {
            source,
            handler,
            is_once,
            is_async,
            span,
        } => Stmt::EventSubscribe {
            source: subst_expr(source, type_map),
            handler: subst_expr(handler, type_map),
            is_once: *is_once,
            is_async: *is_async,
            span: span.clone(),
        },
        Stmt::EventUnsubscribe {
            source,
            handler,
            span,
        } => Stmt::EventUnsubscribe {
            source: subst_expr(source, type_map),
            handler: subst_expr(handler, type_map),
            span: span.clone(),
        },
    }
}
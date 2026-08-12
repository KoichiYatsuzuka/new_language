// stmt/check.rs — 文の静的型検査の中核: check_stmts / check_stmt。

use {
    crate::ast::{Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind},
    crate::type_check::types::InferredType,
    crate::type_check::BinOperandKind,
    crate::type_check::TypeChecker,
};

impl TypeChecker {
    /// 文のスライスを順に型検査する。
    pub(crate) fn check_stmts(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            self.check_stmt(stmt);
        }
    }

    /// 単一の文を型検査する。変数宣言・代入・制御構文・定義文・例外処理・import を網羅する。
    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            // --- 変数宣言 ---
            // Let / Const は不変、Mut は可変。それ以外のロジックは共通。
            Stmt::Let(name, type_ann, expr) | Stmt::Const(name, type_ann, expr) => {
                self.check_var_decl(name, type_ann.as_deref(), expr, stmt, false);
            }
            Stmt::Mut(name, type_ann, expr) => {
                self.check_var_decl(name, type_ann.as_deref(), expr, stmt, true);
            }
            Stmt::Static(name, expr, _) => {
                let ty = self.infer(expr);
                self.declare(name.clone(), ty, true);
            }
            Stmt::LetTuple {
                targets,
                value,
                span,
            } => self.check_let_tuple(targets, value, span),

            // --- 代入 ---
            Stmt::Assign { name, value, span, .. } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                let rhs_ty = self.infer(value);
                if rhs_ty == InferredType::Undefined {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: Some(span.clone()),
                    });
                }
            }
            Stmt::CompoundAssign {
                name,
                op: _,
                value,
                span,
                node_id,
                ..
            } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                }
                let lt = self.lookup(name).map(|i| i.ty.clone());
                let rt = self.infer(value);
                // ── AST 型解決層（#16 / #2b）── `x <op>= e` は `x <op> e` と同じ二項演算なので、
                // `Expr::BinOp` と同じ基準でオペランド種別を焼き、VM が型特化 op を選べるようにする。
                // 焼かないと複合代入だけが汎用 `Bin` に落ちる（実測 1.9x 遅い）。
                if let Some(k) = lt.as_ref().and_then(|lt| BinOperandKind::of(lt, &rt)) {
                    self.annotations.set_binop_kind(*node_id, k);
                }
            }
            // 通常・複合いずれの属性/添字代入も検査内容は同一（複合の op は型に影響しない）。
            Stmt::AttrAssign { target, value }
            | Stmt::AttrCompoundAssign { target, value, .. } => {
                self.check_attr_assign(target, value);
            }

            // --- 式文 ---
            Stmt::Expr(expr) => {
                self.infer(expr);
            }

            // --- 制御構文 ---
            Stmt::If {
                branches,
                else_body,
            } => self.check_if(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.check_match(subject, arms),
            Stmt::While { cond, body } => {
                self.infer(cond);
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::For {
                targets,
                iter,
                body,
            } => {
                let iter_ty = self.infer(iter);
                let elem_ty = Self::for_element_type(&iter_ty);
                self.push_scope();
                // 分割代入（`for k, v in pairs`）は要素がタプルのときだけ各要素型へ割り当てる。
                let target_tys: Vec<InferredType> = match (&elem_ty, targets.len()) {
                    (_, 1) => vec![elem_ty.clone()],
                    (InferredType::Tuple(ts), n) if ts.len() == n => ts.clone(),
                    (_, n) => vec![InferredType::Unresolved; n],
                };
                for (t, ty) in targets.iter().zip(target_tys) {
                    self.declare(t.clone(), ty, true);
                }
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::Block(body) => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }

            // --- 関数定義 ---
            Stmt::FnDef {
                name,
                params,
                return_type,
                body,
                decorators,
                ..
            } => self.check_fn_def(name, params, return_type.as_deref(), body, decorators),

            // --- クラス・trait 定義 ---
            Stmt::ClassDef {
                name,
                body,
                decorators,
                ..
            } => {
                for dec in decorators {
                    self.check_decorator(dec, false, name);
                }
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                let prev_class = self.state.enter_class(name.clone());
                self.check_stmts(body);
                self.state.exit_class(prev_class);
                self.pop_scope();
            }
            Stmt::TraitDef { name, body, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::ProtocolDef { name, .. } => {
                // プロトコルは型値としてスコープに登録する（インスタンス化試行を検出するため）
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::Protocol(name.clone()))),
                    false,
                );
                // collect_fn_sigs で already registered in known_protocols
            }

            // --- ジャンプ文 ---
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.infer(e);
                }
            }
            Stmt::BlockReturn(expr, span) => {
                if self.state.block_return_forbidden() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::BlockReturnInLoopExpr,
                        span: Some(span.clone()),
                    });
                }
                self.infer(expr);
            }
            Stmt::LoopYield(expr) | Stmt::Yield(expr) => {
                self.infer(expr);
            }

            // --- クラスフィールド宣言 ---
            Stmt::Field {
                name,
                kind,
                type_ann,
                default,
                ..
            } => {
                let ty = InferredType::from_ann(type_ann).unwrap_or(InferredType::Unresolved);
                if let Some(expr) = default {
                    if matches!(kind, FieldKind::Mut | FieldKind::Let) {
                        let kind_str = if matches!(kind, FieldKind::Mut) { "mut" } else { "let" };
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::FieldDefaultNotAllowed {
                                field_name: name.clone(),
                                kind: kind_str.to_string(),
                            },
                            span: None,
                        });
                    }
                    self.infer(expr);
                }
                let mutable = matches!(kind, FieldKind::Mut);
                self.declare(name.clone(), ty, mutable);
            }

            // --- ジェネレータ関数定義 ---
            Stmt::GenDef {
                name,
                params,
                yield_type,
                body,
                ..
            } => self.check_gen_def(name, params, yield_type.as_deref(), body),

            // --- new_type 定義 ---
            Stmt::NewTypeDef { name, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- enum 定義 ---
            Stmt::EnumDef { name, .. } => {
                let item_type_name = format!("enum_item_{}", name);
                self.declare(
                    item_type_name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(item_type_name))),
                    false,
                );
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- 副作用のない文 ---
            Stmt::Pass
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Freeze(..)
            | Stmt::BreakPoint { .. }
            | Stmt::DebugLet(..) => {}

            // --- 例外処理 ---
            Stmt::Try {
                body,
                handlers,
                finally_body,
            } => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                for handler in handlers {
                    self.push_scope();
                    if let Some(name) = &handler.name {
                        self.declare(name.clone(), InferredType::Unresolved, true);
                    }
                    self.check_stmts(&handler.body);
                    self.pop_scope();
                }
                if let Some(fb) = finally_body {
                    self.push_scope();
                    self.check_stmts(fb);
                    self.pop_scope();
                }
            }
            Stmt::Raise { exc, span } => {
                if let Some(e) = exc {
                    let ty = self.infer(e);
                    if !self.is_error_instance_type(&ty) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::InvalidRaiseType { got: ty },
                            span: Some(span.clone()),
                        });
                    }
                }
            }

            // --- import ---
            Stmt::Import {
                lang,
                module,
                alias,
                body,
                ..
            } => {
                self.annotate_module_body(lang, module, body);
                let member_types = self.collect_module_types(body);
                let bind_name = alias
                    .clone()
                    .unwrap_or_else(|| module.last().unwrap().clone());
                let ns_ty = if lang == "py" || lang == "py-int" {
                    InferredType::PyNamespace(member_types)
                } else {
                    InferredType::Namespace(member_types)
                };
                self.declare(bind_name, ns_ty, false);
            }

            Stmt::FromImport { lang, module, names, body, .. } => {
                self.annotate_module_body(lang, module, body);
                let member_types = self.collect_module_types(body);
                let is_py = lang == "py" || lang == "py-int";
                for (orig_name, alias) in names {
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    let ty = member_types
                        .get(orig_name.as_str())
                        .cloned()
                        .unwrap_or(if is_py { InferredType::Any } else { InferredType::Unresolved });
                    self.declare(bind_name, ty, false);
                }
            }

            Stmt::AsyncAssign { stmts, .. } => {
                self.push_scope();
                self.check_stmts(stmts);
                self.pop_scope();
            }

            Stmt::EventSubscribe { .. } | Stmt::EventUnsubscribe { .. } => {
                // イベント購読/解除文: 現時点では型チェックをスキップ
            }
        }
    }

    /// `match` 文を型検査する。`is Type` パターンでは対象変数を各腕スコープ内で絞り込む。
    fn check_match(&mut self, subject: &Expr, arms: &[MatchArm]) {
        let subject_ty = self.infer(subject);
        let subject_name: Option<String> = if let Expr::Ident { name: n, .. } = subject {
            Some(n.clone())
        } else {
            None
        };
        for arm in arms {
            self.push_scope();
            match &arm.pattern {
                MatchPattern::Case(expr) => {
                    self.infer(expr);
                }
                MatchPattern::IsType(type_name) => {
                    if let Some(ref var_name) = subject_name {
                        let narrowed = Self::type_from_guard_name(type_name);
                        let is_mut = self.lookup(var_name).map(|v| v.mutable).unwrap_or(false);
                        self.declare(var_name.clone(), narrowed, is_mut);
                    }
                    let _ = subject_ty.clone();
                }
            }
            self.check_stmts(&arm.body);
            self.pop_scope();
        }
    }

    /// `if` / `elif` / `else` を型検査する。各分岐の条件が型ガード
    /// (`is Type` / `x.is_OK()` / `x.is_ERR()`) のとき、その分岐スコープ内で
    /// 対象変数の型を絞り込む。
    fn check_if(&mut self, branches: &[(Expr, Vec<Stmt>)], else_body: &Option<Vec<Stmt>>) {
        for (cond, body) in branches {
            let guard_opt = Self::detect_type_guard(cond);
            // Result 型ガード (`x.is_OK()` / `x.is_ERR()`) は type_guard より優先する。
            let result_guard = self.detect_result_guard(cond);
            let (narrowed, error_info) = self.narrow_by_type_guard(guard_opt);

            self.infer(cond);

            if let Some((var_name, var_type, span)) = error_info {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::IsNotOnNonUnion { var_name, var_type },
                    span: Some(span),
                });
            }

            self.push_scope();
            if let Some((var_name, narrowed_ty, is_mut)) = result_guard.or(narrowed) {
                self.declare(var_name, narrowed_ty, is_mut);
            }
            self.check_stmts(body);
            self.pop_scope();
        }
        if let Some(body) = else_body {
            self.push_scope();
            self.check_stmts(body);
            self.pop_scope();
        }
    }

    /// 条件式が `is Type` ガードなら `(変数名, 型名, 否定か, span)` を返す。
    fn detect_type_guard(cond: &Expr) -> Option<(String, String, bool, Span)> {
        let Expr::IsType { expr, type_name, negated, span, .. } = cond else {
            return None;
        };
        let Expr::Ident { name: var_name, .. } = expr.as_ref() else {
            return None;
        };
        Some((var_name.clone(), type_name.clone(), *negated, span.clone()))
    }

    /// 条件式が Result 型ガード (`x.is_OK()` / `x.is_ERR()`) なら、絞り込んだ
    /// `(変数名, 絞り込み後の型, 可変か)` を返す。
    fn detect_result_guard(&self, cond: &Expr) -> Option<(String, InferredType, bool)> {
        let Expr::Call { func, args, .. } = cond else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let Expr::Attr { object, attr, .. } = func.as_ref() else {
            return None;
        };
        if attr != "is_OK" && attr != "is_ERR" {
            return None;
        }
        let Expr::Ident { name: var_name, .. } = object.as_ref() else {
            return None;
        };
        let info = self.lookup(var_name)?;
        let InferredType::Result(ok_ty, err_ty) = &info.ty else {
            return None;
        };
        let narrowed_ty = if attr == "is_OK" {
            *ok_ty.clone()
        } else {
            *err_ty.clone()
        };
        Some((var_name.clone(), narrowed_ty, info.mutable))
    }

    /// `is Type` ガードから分岐スコープの絞り込みを計算する。
    /// 戻り値は `(絞り込み束縛, `is not` を非 Union に使ったときのエラー情報)`。
    #[allow(clippy::type_complexity)]
    fn narrow_by_type_guard(
        &mut self,
        guard_opt: Option<(String, String, bool, Span)>,
    ) -> (
        Option<(String, InferredType, bool)>,
        Option<(String, InferredType, Span)>,
    ) {
        let Some((var_name, type_name, negated, span)) = guard_opt else {
            return (None, None);
        };
        let guard_ty = if self.registry.is_protocol(type_name.as_str()) {
            InferredType::Protocol(type_name.clone())
        } else {
            Self::type_from_guard_name(&type_name)
        };
        let (var_ty, is_mut) = self
            .lookup(&var_name)
            .map(|v| (v.ty.clone(), v.mutable))
            .unwrap_or((InferredType::Unresolved, false));

        if negated {
            match &var_ty {
                InferredType::Union(types) => {
                    let remaining: Vec<InferredType> =
                        types.iter().filter(|t| **t != guard_ty).cloned().collect();
                    let narrowed_ty = match remaining.len() {
                        0 => InferredType::Unresolved,
                        1 => remaining.into_iter().next().unwrap(),
                        _ => InferredType::Union(remaining),
                    };
                    (Some((var_name, narrowed_ty, is_mut)), None)
                }
                InferredType::Unresolved => (None, None),
                _ => (None, Some((var_name, var_ty.clone(), span))),
            }
        } else {
            // `is TypeName` guard: var_ty が交差型ならガード型の適合を検証する
            if let InferredType::Intersection(isect_types) = &var_ty {
                let isect_cloned = isect_types.clone();
                self.check_intersection_guard_type(&type_name, &isect_cloned, Some(span));
            }
            (Some((var_name, guard_ty, is_mut)), None)
        }
    }

    /// 属性/添字への代入 (`obj.attr = v` / `a[i] = v` と複合代入版) を型検査する。
    /// 添字代入のルートが不変変数ならエラー、不変フィールドへの代入もエラーにする。
    fn check_attr_assign(&mut self, target: &Expr, value: &Expr) {
        if matches!(target, Expr::Subscript { .. }) {
            if let Some(name) = Self::subscript_root_ident(target) {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::AssignToImmutable { name: name.to_string() },
                            span: None,
                        });
                    }
                }
            }
        }
        self.check_immutable_field_assign(target);
        self.infer(target);
        self.infer(value);
    }

    /// タプル分割束縛 `let a, b = expr` を型検査する。
    fn check_let_tuple(&mut self, targets: &[TupleTarget], value: &Expr, span: &Span) {
        let rhs_ty = self.infer(value);

        for target in targets.iter() {
            if let TupleTarget::Bare(name) = target {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::TupleUnpackMissingQualifier { name: name.clone() },
                    span: Some(span.clone()),
                });
            }
        }

        if let InferredType::Tuple(ref elem_types) = rhs_ty {
            let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
            let named = targets
                .iter()
                .filter(|t| !matches!(t, TupleTarget::Wildcard))
                .count();
            let tlen = elem_types.len();
            let bad = if has_wildcard { named > tlen } else { named != tlen };
            if bad {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::TupleUnpackArityMismatch {
                        tuple_len: tlen,
                        target_count: named,
                        has_wildcard,
                    },
                    span: Some(span.clone()),
                });
            }
        }

        let elem_types = if let InferredType::Tuple(ref v) = rhs_ty {
            v.clone()
        } else {
            vec![]
        };
        for (i, target) in targets.iter().enumerate() {
            let ty = elem_types.get(i).cloned().unwrap_or(InferredType::Any);
            let (name, mutable) = match target {
                TupleTarget::Let(name) | TupleTarget::Bare(name) => (name, false),
                TupleTarget::Mut(name) => (name, true),
                TupleTarget::Wildcard => continue,
            };
            if name != "_" && self.lookup(name).is_some() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                    span: Some(span.clone()),
                });
            }
            self.declare(name.clone(), ty, mutable);
        }
    }

    /// 関数定義を型検査する。パラメータ・戻り値の注釈欠落や交差型を診断し、
    /// パラメータをスコープに束縛して本体を検査する。
    fn check_fn_def(
        &mut self,
        name: &str,
        params: &[Param],
        return_type: Option<&str>,
        body: &[Stmt],
        decorators: &[Expr],
    ) {
        for dec in decorators {
            self.check_decorator(dec, true, name);
        }
        for param in params.iter() {
            if param.name == "self" || param.variadic {
                continue;
            }
            if param.type_ann.is_none() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::MissingParamTypeAnn {
                        func_name: name.to_string(),
                        param_name: param.name.clone(),
                    },
                    span: None,
                });
            }
        }
        match return_type {
            None => self.report_error(StaticTypeError {
                kind: TypeErrorKind::MissingReturnTypeAnn { func_name: name.to_string() },
                span: None,
            }),
            Some(rt) if self.registry.is_protocol(rt) => {
                self.report_warning(StaticTypeWarning {
                    kind: TypeWarningKind::ProtocolReturnType {
                        func_name: name.to_string(),
                        protocol_name: rt.to_string(),
                    },
                    span: None,
                });
            }
            Some(_) => {}
        }
        // 交差型を含む関数は部分コンパイルできないため警告を出す
        let has_intersection_type = params.iter().any(|p| {
            p.type_ann.as_deref()
                .and_then(InferredType::from_ann)
                .is_some_and(|ty| matches!(ty, InferredType::Intersection(_)))
        }) || return_type
            .and_then(InferredType::from_ann)
            .is_some_and(|ty| matches!(ty, InferredType::Intersection(_)));
        if has_intersection_type {
            self.report_warning(StaticTypeWarning {
                kind: TypeWarningKind::IntersectionSkippedCompile { func_name: name.to_string() },
                span: None,
            });
        }
        self.declare(name.to_string(), InferredType::Unresolved, false);
        self.push_scope();
        for param in params {
            self.declare_param(param);
        }
        let prev_fn = self.state.enter_fn(name.to_string());
        self.with_barrier(|c| c.check_stmts(body));
        self.state.exit_fn(prev_fn);
        self.pop_scope();
    }

    /// ジェネレータ関数定義を型検査する。
    fn check_gen_def(
        &mut self,
        name: &str,
        params: &[Param],
        yield_type: Option<&str>,
        body: &[Stmt],
    ) {
        for param in params.iter() {
            if param.name == "self" || param.variadic {
                continue;
            }
            if param.type_ann.is_none() {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::MissingParamTypeAnn {
                        func_name: name.to_string(),
                        param_name: param.name.clone(),
                    },
                    span: None,
                });
            }
        }
        if yield_type.is_none() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::MissingReturnTypeAnn { func_name: name.to_string() },
                span: None,
            });
        }
        self.declare(name.to_string(), InferredType::Unresolved, false);
        self.push_scope();
        for param in params {
            let ty = param
                .type_ann
                .as_deref()
                .and_then(InferredType::from_ann)
                .unwrap_or(InferredType::Unresolved);
            self.declare(param.name.clone(), ty, param.mutable);
        }
        self.with_barrier(|c| c.check_stmts(body));
        self.pop_scope();
    }

    /// 関数パラメータ1つをスコープに束縛する（`self`・可変長・通常引数を区別）。
    fn declare_param(&mut self, param: &Param) {
        if param.variadic {
            // 可変長パラメータ: local::args として Optional[list[T]] を宣言
            let elem_ty = param
                .type_ann
                .as_deref()
                .and_then(InferredType::from_ann)
                .unwrap_or(InferredType::Any);
            let local_args_ty = InferredType::Union(vec![
                InferredType::ListOf(Box::new(elem_ty)),
                InferredType::None,
            ]);
            self.declare("local::args".to_string(), local_args_ty, param.mutable);
            return;
        }
        let ty = if param.name == "self" {
            self.state
                .current_class()
                .map(|c| InferredType::NamedInstance(c.to_string()))
                .unwrap_or(InferredType::Unresolved)
        } else {
            param
                .type_ann
                .as_deref()
                .and_then(InferredType::from_ann)
                .unwrap_or(InferredType::Unresolved)
        };
        self.declare(param.name.clone(), ty, param.mutable);
    }

    /// `let` / `const` / `mut` 宣言の共通処理。`mutable` だけが3者で異なる。
    fn check_var_decl(
        &mut self,
        name: &str,
        type_ann: Option<&str>,
        expr: &Expr,
        stmt: &Stmt,
        mutable: bool,
    ) {
        let rhs_ty = self.infer(expr);
        if rhs_ty == InferredType::Undefined {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::AssignUndefined,
                span: None,
            });
        }
        if name != "_" && self.lookup(name).is_some() {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::VariableRedeclaration { name: name.to_string() },
                span: None,
            });
        }
        let ty = self.resolve_declared_type(type_ann, rhs_ty, name, stmt);
        self.declare(name.to_string(), ty, mutable);
    }
}

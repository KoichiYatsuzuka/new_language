// stmt/check/bindings.rs — 変数の束縛の検査: 変数宣言・組の分解・Python の物の `mut`。

use super::*;

impl TypeChecker {
    /// タプル分割束縛 `let a, b = expr` を型検査する。
    pub(super) fn check_let_tuple(&mut self, targets: &[TupleTarget], value: &Expr, span: &Span) {
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
            if name != "_" && self.already_declared(name) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::VariableRedeclaration { name: name.clone() },
                    span: Some(span.clone()),
                });
            }
            if !mutable {
                self.require_mut_for_python_object(name, "let", &ty);
            }
            self.note_binding(name, &ty);
            self.declare(name.clone(), ty, mutable);
        }
    }

    /// `let` / `const` / `mut` 宣言の共通処理。`mutable` だけが3者で異なる。
    pub(super) fn check_var_decl(
        &mut self,
        name: &str,
        type_ann: Option<&str>,
        expr: &Expr,
        stmt: &Stmt,
        mutable: bool,
    ) {
        let rhs_ty = self.infer(expr);
        // 値の誤りは**右辺**を指す（diagnostic_position_plan.md の 3-3）。錨が無ければ文の位置。
        if rhs_ty == InferredType::Undefined {
            self.report_error_at(expr, StaticTypeError {
                kind: TypeErrorKind::AssignUndefined,
                span: None,
            });
        }
        // ⚠⚠ `Code` の持ち出し禁止（設計書 §1.2 / タスク 1-5）。
        //   ⚠ **メタ関数の中かどうかを見る必要はない。** `check_stmt` の `MetaFnDef` は
        //     本体へ降りない（本体は展開時に走るので通常の型検査の対象外）ので、
        //     ここで `Code` が見えたということ自体が「外へ出た」ことを意味する。
        //   ⚠ 本体を検査するようにしたら**ここに深さの条件を足すこと**。
        if rhs_ty == InferredType::Code {
            self.report_error_at(expr, StaticTypeError {
                kind: TypeErrorKind::CodeEscapesMetafunction { name: name.to_string() },
                span: None,
            });
        }
        if name != "_" && self.already_declared(name) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::VariableRedeclaration { name: name.to_string() },
                span: None,
            });
        }
        let ty = self.resolve_declared_type(type_ann, rhs_ty, name, stmt, expr);
        if !mutable {
            let binding = if matches!(stmt, Stmt::Const(..)) { "const" } else { "let" };
            self.require_mut_for_python_object(name, binding, &ty);
        }
        self.note_binding(name, &ty);
        self.declare(name.to_string(), ty, mutable);
    }

    /// Python のオブジェクトを持ちうる型か（pandas_import_plan.md の D-3・1-2）。コンテナの要素も見る（`list[pd.DataFrame]`）。
    ///
    /// ⚠ 静的に分かるのは、CPython で実行するモジュールのクラスのインスタンスだけ。Python の関数を値として読んだもの
    ///   （型は関数）や `Any` の値は、実行時の検査（`Interpreter::bind_copy_held` / 引数の束縛）が止める。
    pub(crate) fn holds_cpython_object(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(n) => self.registry.is_cpython_class(n),
            InferredType::GenericInstance { name, args } => {
                self.registry.is_cpython_class(name) || args.iter().any(|a| self.holds_cpython_object(a))
            }
            InferredType::ListOf(e)
            | InferredType::FixedListOf(e)
            | InferredType::ListLikeOf(e)
            | InferredType::SetOf(e)
            | InferredType::FrozenSetOf(e)
            | InferredType::IteratorOf(e) => self.holds_cpython_object(e),
            InferredType::DictOf(k, v) => self.holds_cpython_object(k) || self.holds_cpython_object(v),
            // 中身が CPython の物（D-12・2-7）
            InferredType::PyList(_) | InferredType::PyDict(..) | InferredType::PySet(_) => true,
            InferredType::Tuple(items) | InferredType::Union(items) => {
                items.iter().any(|t| self.holds_cpython_object(t))
            }
            _ => false,
        }
    }

    /// 不変の束縛（`binding`）が Python のオブジェクトを受けるなら誤り（D-3・1-2）。
    pub(crate) fn require_mut_for_python_object(&mut self, name: &str, binding: &'static str, ty: &InferredType) {
        if self.holds_cpython_object(ty) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ImmutablePythonObject { name: name.to_string(), binding },
                span: None,
            });
        }
    }
}

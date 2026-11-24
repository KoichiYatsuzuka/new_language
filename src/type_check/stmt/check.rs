// stmt/check.rs — 文の静的型検査の中核: check_stmts / check_stmt。

use {
    crate::ast::{BinOp, Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind},
    crate::type_check::types::{ClosedModule, InferredType},
    crate::type_check::BinOperandKind,
    crate::type_check::TypeChecker,
};

mod branches;
mod assign;
mod functions;
mod field_types;
mod bindings;

/// Arrow のモジュールなら、**メンバーが確定している**ことの印（[`ClosedModule`]）を作る（2026-10-02）。
///
/// ⚠ メンバーは最上位で束縛される名前すべて（宣言・import の束縛＝再エクスポート・CPython 準拠）と、
///   どこかで import されるサブモジュール（`TypeChecker::namespace_type`）。
/// ⚠ 外部言語のスタブ（cpp / cs / js / rs）は実物の部分集合でありうるので作らない。
/// ⚠ 拡張（`editor`）も CLI と同じに作る（3-3）。以前は拡張だけ import 先を読まず、ホストが渡すスタブが
///   実モジュールの部分集合だったので作らなかった。読めなかった import（`ImportOrigin::unresolved`）の
///   モジュールは呼び出し側が作らない。
pub(crate) fn closed_module(lang: &str, module: &[String]) -> Option<Box<ClosedModule>> {
    if !crate::module_path::is_arrow_source_lang(lang) || module.is_empty() {
        return None;
    }
    Some(Box::new(ClosedModule { name: module.join(".") }))
}

/// C/C++ のモジュールの本体（型スタブ）から、今の Arrow から正しく呼べない関数の `(名前, 理由)` を集める
/// （ffi_value_exchange_plan.md 1-2）。パーサが理由を書いた文字列のデコレータ（`FFI_UNSUPPORTED_MARK` で始まる）
/// を付けている（`parser/imports/cpp.rs`）。
fn ffi_unsupported_fns(body: &[Stmt]) -> Vec<(String, String)> {
    body.iter()
        .filter_map(|st| match st {
            Stmt::FnDef { name, .. } => {
                crate::type_check::ffi_unsupported_reason(st).map(|reason| (name.clone(), reason))
            }
            _ => None,
        })
        .collect()
}

impl TypeChecker {
    /// 文のスライスを順に型検査する。
    pub(crate) fn check_stmts(&mut self, stmts: &[Stmt]) {
        let mut group_end = 0;
        for (i, stmt) in stmts.iter().enumerate() {
            // 間に文を挟まずに続く `fn` / `gen` の定義は 1 つの組（name_scope_plan.md のフェーズ 3・3-2）。
            // 組の名前とシグネチャを、最初の定義より前に全部宣言しておく（組の中の前方参照・相互再帰を通す）。
            if i >= group_end && !self.state.in_class_body() {
                let len = crate::decl_names::fn_group_len(&stmts[i..]);
                group_end = i + len;
                if len >= 2 {
                    self.predeclare_fn_group(&stmts[i..group_end]);
                }
            }
            self.check_stmt(stmt);
        }
    }

    /// 組（`decl_names::fn_group_len`）の関数の名前とシグネチャを先に宣言する。各定義の検査（`check_fn_def` / `check_gen_def`）は
    /// `group_predeclared` に残った名前の宣言を飛ばす（同じスコープに 2 回積まない）。
    /// ⚠ 組の外の名前は今までどおり定義の時点のスコープで引く（D-1 = (a)）。
    fn predeclare_fn_group(&mut self, group: &[Stmt]) {
        for st in group {
            let (name, params, ret) = match st {
                Stmt::FnDef { name, params, return_type, template_params, .. } if template_params.is_empty() => (
                    name,
                    params,
                    return_type.as_deref().and_then(InferredType::from_ann).map(|t| self.registry.resolve_protocol_ann(t)),
                ),
                Stmt::GenDef { name, params, yield_type, template_params, .. } if template_params.is_empty() => (
                    name,
                    params,
                    yield_type
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .map(|t| InferredType::IteratorOf(Box::new(self.registry.resolve_protocol_ann(t)))),
                ),
                _ => continue,
            };
            if self.state.in_deferred_body() {
                let sig = self.registry.sig_of(params, ret);
                self.state.declare_local_fn(name.clone(), sig);
            }
            self.group_predeclared.push(name.clone());
        }
        for name in self.group_predeclared.clone() {
            let ty = self.fn_value_type(&name).unwrap_or(InferredType::Unresolved);
            self.declare(name, ty, false);
        }
    }

    /// 単一の文を型検査する。変数宣言・代入・制御構文・定義文・例外処理・import を網羅する。
    ///
    /// ⚠ 検査の間は**今の文の位置**を控える（フェーズ10 10-17）。位置を持たない誤りのうち、推論中の式の
    ///   錨（`expr_anchor`）が無いもの（推論が終わってから出す `let z: str = s` の不一致など）はこの位置で
    ///   報告する（`report_error`）。位置を持たない文（`pass` など）は外側の文の位置を引き継ぐ。
    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) {
        // 式文は文の位置を持たないので、式そのものの位置（呼び出しの `(` など）で代える。
        let here = stmt.position().or_else(|| match stmt {
            Stmt::Expr(e) => e.own_span(),
            _ => None,
        });
        let outer = match here {
            Some(p) => self.stmt_pos.replace(p.clone()),
            None => self.stmt_pos.clone(),
        };
        // 式の中の文（`if` 式の本体など）は外側の式の錨を引き継がない（diagnostic_position_plan.md の 2-1）。
        let outer_anchor = self.expr_anchor.take();
        // 束縛の記録に付ける位置（`bind_pos` の doc）。`Stmt::position` だけを見る。
        // 記録しない検査（CLI）では張り替えない。
        let outer_bind = match (self.bindings.is_some(), stmt.position()) {
            (true, Some(p)) => Some(self.bind_pos.replace(p.clone())),
            _ => None,
        };
        self.check_stmt_inner(stmt);
        if let Some(prev) = outer_bind {
            self.bind_pos = prev;
        }
        self.stmt_pos = outer;
        self.expr_anchor = outer_anchor;
    }

    fn check_stmt_inner(&mut self, stmt: &Stmt) {
        match stmt {
            // --- 変数宣言 ---
            // Let / Const は不変、Mut は可変。それ以外のロジックは共通。
            Stmt::Let(name, type_ann, expr, _) | Stmt::Const(name, type_ann, expr, _) => {
                self.check_var_decl(name, type_ann.as_deref(), expr, stmt, false);
            }
            Stmt::Mut(name, type_ann, expr, _) => {
                self.check_var_decl(name, type_ann.as_deref(), expr, stmt, true);
            }
            Stmt::Static(name, expr, _) => {
                let ty = self.infer(expr);
                self.note_binding(name, &ty);
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
                } else {
                    // 見えない名前への代入（name_scope_plan.md の 1-3）。実行時は `NameError`。読みと同じ判定
                    // （`check_name_defined`）を通す。以前は代入の左辺だけ検査が無く、`opened += 1`
                    // （クラスの `static mut` を裸で書く）が静的に通っていた。
                    let _ = self.check_name_defined(name);
                }
                // ⚠ `freeze` 済みの名前への再束縛も**上の `!info.mutable` で捕まる**
                //    （タスク 9.2 で `freeze` を可変フラグの降格にしたため）。
                //    以前はここに `is_frozen` の別検査があったが、同じ問いに 2 つの
                //    実装がある状態だったので撤去した。
                let rhs_ty = self.infer(value);
                // 値の誤りは**右辺**を指す（diagnostic_position_plan.md の 3-3）。右辺が錨を持たなければ文の位置
                // （＝左辺の名前）。
                if rhs_ty == InferredType::Undefined {
                    self.report_error_at(value, StaticTypeError {
                        kind: TypeErrorKind::AssignUndefined,
                        span: None,
                    });
                }
                // ── 再代入の型検査（0-7）────────────────────────────────────
                // ⚠ 0-1 で「変数を**注釈の型で束縛する**」ようにして初めて成立する。
                //    それ以前は束縛型が右辺の推論型だったので、照合しても意味が無かった。
                // ⚠ protocol 型の変数への再代入もここを通る（`check_expected` が
                //    適合検査へ回す）。ここが無いと `mut x: Pr = Good(1); x = Bad(...)` が
                //    素通りする（実測）。
                // ⚠⚠ **注釈の有無で緩めない。** Arrow では型注釈は補助的なもので、
                //    注釈が無い束縛にも推論型で同じ厳格さを適用する。
                //      mut y = [1]   # y は list[int]
                //      y = [y]       # list[list[int]] ⇒ エラー
                //    検査を緩めて例題の失敗を避けるのではなく、例題側を直す
                //    （`equality_depth_limit_error.ar` は `mut y: list` へ直した）。
                let declared = self.lookup(name).map(|i| i.ty.clone());
                if let Some(declared) = declared {
                    if !matches!(declared, InferredType::Unresolved | InferredType::Any)
                        && !self.mentions_type_param(&declared)
                    {
                        let ctx = format!("variable `{name}`");
                        if !self.with_anchor_at(value, |c| c.check_expected(&rhs_ty, &declared, false, &ctx)) {
                            self.report_error_at(value, StaticTypeError {
                                kind: TypeErrorKind::VarTypeMismatch {
                                    name: name.clone(),
                                    expected: declared,
                                    got: rhs_ty,
                                },
                                span: None,
                            });
                        }
                    }
                }
            }
            Stmt::CompoundAssign {
                name,
                op,
                value,
                span,
                node_id,
                ..
            } => {
                if let Some(info) = self.lookup(name) {
                    if !info.mutable {
                        self.report_error(StaticTypeError::assign_immutable(name, span.clone()));
                    }
                } else {
                    // 見えない名前への代入（name_scope_plan.md の 1-3）。実行時は `NameError`。読みと同じ判定
                    // （`check_name_defined`）を通す。以前は代入の左辺だけ検査が無く、`opened += 1`
                    // （クラスの `static mut` を裸で書く）が静的に通っていた。
                    let _ = self.check_name_defined(name);
                }
                let lt = self.lookup(name).map(|i| i.ty.clone());
                let rt = self.infer(value);
                // ⚠⚠ **二段検査**（D-8・タスク 5.1）。`x + "s"` は静的エラーなのに
                //    `x += "s"` は実行時まで判らなかった（検体 `B6`）。
                if let Some(lt) = lt.as_ref() {
                    self.check_compound_assign(name, op, lt, &rt, Some(span.clone()));
                }
                // ── AST 型解決層（#16 / #2b）── `x <op>= e` は `x <op> e` と同じ二項演算なので、
                // `Expr::BinOp` と同じ基準でオペランド種別を焼き、VM が型特化 op を選べるようにする。
                // 焼かないと複合代入だけが汎用 `Bin` に落ちる（実測 1.9x 遅い）。
                if let Some(k) = lt.as_ref().and_then(|lt| BinOperandKind::of(lt, &rt)) {
                    self.annotations.set_binop_kind(*node_id, k);
                }
            }
            // 可変性・アクセス制御の検査は通常・複合で同一。
            // ⚠ **フィールドの型検査の仕方が違う**（0-2）。複合代入で格納されるのは
            //    `v` ではなく `f <op> v` の結果で、その型は二項演算の規則で決まる。
            //    `v` をそのままフィールド型と突き合わせると嘘の判定になる。
            //    ⇒ タスク 5.1 で**二段検査**（D-8）に置き換えた。以前はここで
            //      検査を**丸ごと省いて**いたので `c.n += "s"` が実行時まで判らなかった
            //      （検体 `F2`）。
            Stmt::AttrAssign { target, value, span: _ } => {
                self.check_attr_assign(target, value, None);
            }
            Stmt::AttrCompoundAssign { target, op, value, span: _ } => {
                self.check_attr_assign(target, value, Some(op));
            }

            // --- 式文 ---
            // ⚠ 結果を捨てる文なので型義務は無い。
            Stmt::Expr(expr) => {
                self.walk(expr);
            }

            // --- 制御構文 ---
            Stmt::If {
                branches,
                else_body,
                span: _,
            } => self.check_if(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.check_match(subject, arms),
            Stmt::While { cond, body, span: _ } => {
                // ⚠ 条件は `bool` でなければならない（D-12・検体 K2）。
                self.check_condition_is_bool(cond, "while");
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
            }
            Stmt::For {
                targets,
                mut_targets,
                iter,
                body,
                span: _,
            } => {
                let iter_ty = self.infer(iter);
                // ⚠ 反復できない型を弾く（タスク 7.1・検体 `K3`）。
                self.check_iterable(&iter_ty, None);
                let elem_ty = self.iter_element_type(&iter_ty);
                self.note_python_iteration(iter, &iter_ty, &elem_ty);
                self.push_scope();
                // 分割代入（`for k, v in pairs`）は要素がタプルのときだけ各要素型へ割り当てる。
                let target_tys: Vec<InferredType> = match (&elem_ty, targets.len()) {
                    (_, 1) => vec![elem_ty.clone()],
                    (InferredType::Tuple(ts), n) if ts.len() == n => ts.clone(),
                    (_, n) => vec![InferredType::Unresolved; n],
                };
                // ⚠⚠ **ループ変数はコンテナの属性を引き継ぐ**（規則 1・L3）。
                //    以前は無条件に `mutable = true` だったので、
                //      let zs = [[1]]
                //      for it in zs:
                //          it.append(9)      # 通ってしまう
                //    で **`let` の `zs` の要素が変わっていた**（実測）。
                // ⚠⚠ 根が識別子でない反復対象（`range(n)` / リテラル）は**一時値**。
                //    その場で作られて誰とも共有していないが、**だからこそ書き換える意味が無い**
                //    ので `let` 扱いにする（規則 3・B4）。以前は `unwrap_or(true)` だったため
                //      for i in range(3):
                //          i = i * 10      # 通ってしまう（B4 のケース h）
                //    とループ変数そのものを潰せた。
                // `for mut x in ..`（D-11・2-10）は可変の束縛（要素を写す）
                let target_mut = *mut_targets || self.path_is_mutable(iter).unwrap_or(false);
                // ループ変数が「誰からも書き換わらない値」か（反復対象が証明できた経路・3-1）。`for mut` は写しなので対象外
                let iter_proven = !*mut_targets && self.proven_immutable_path(iter);
                self.for_var_proof.push(targets.iter().map(|t| (t.clone(), iter_proven)).collect());
                for (t, ty) in targets.iter().zip(target_tys) {
                    // ⚠ Python の物を受けるループ変数は `for mut` と書く（D-11）。外の束縛が `mut` でも要る（実行時も
                    //   `mut` と書いたかだけで判定する）
                    if !*mut_targets && t != "_" {
                        self.require_mut_for_python_object(t, "for", &ty);
                    }
                    // ⚠⚠ ループ変数は**ブロック内の新しい束縛**（規則 1・B4）。
                    //    外側に同名があれば**再束縛**であり、`let` / `mut` の再宣言と
                    //    同じくエラーにする。以前は黙って覆うだけだったので
                    //      mut i = -1
                    //      for i in range(3): ...
                    //      print(i)          # -1（ループの i ではない）
                    //    という読み違えやすい形が通っていた（B4 のケース a/d/e/f）。
                    // ⚠ 検査は `Stmt::Let` / タプル展開と**同じ形**に揃えてある
                    //    （`_` は除外・素の `lookup` なので組み込み名も対象）。
                    //    新しいエラー種別を作らないのは、これが新規の規則ではなく
                    //    **既存の再宣言規則を for ターゲットにも及ぼしただけ**だから。
                    if t != "_" && self.already_declared(t) {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::VariableRedeclaration { name: t.clone() },
                            span: None,
                        });
                    }
                    self.note_binding(t, &ty);
                    self.declare(t.clone(), ty, target_mut);
                }
                self.check_stmts(body);
                self.for_var_proof.pop();
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
                template_params,
                ..
            } => self.check_fn_def(
                name,
                params,
                return_type.as_deref(),
                body,
                decorators,
                template_params,
            ),

            // --- クラス・trait 定義 ---
            Stmt::ClassDef {
                name,
                body,
                decorators,
                template_params,
                ..
            } => {
                for dec in decorators {
                    // 装飾子の誤りは装飾子の式を指す（diagnostic_position_plan.md の 3-4）。
                    self.with_anchor_at(dec, |c| c.check_decorator(dec, false, name));
                }
                // ⚠ クラスの型変数はメソッド本体からも見える（`mut v: T` を `self.v` で書く）。
                let saved_tp =
                    self.state.push_type_params(template_params.iter().map(|p| p.name.clone()));
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                self.push_scope();
                self.state.enter_class_body();
                let prev_class = self.state.enter_class(name.clone());
                // ⚠ 仮想メソッド（`...` 本体）は **trait 専用**。クラスに置くと実装を強制する
                //    相手が居ないので、以前は黙って `None` を返す no-op になっていた（0-10）。
                // ⚠⚠ 外部言語のスタブクラスは `...` 本体を正当に使う（本体が向こう側にある）
                //    ので、**Arrow ソースのクラスだけ**を対象にする（#27-a）。
                if self.registry.arrow_class_names().contains(name) {
                    for st in body {
                        if let Stmt::FnDef { name: mname, is_abstract: true, .. } = st {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::VirtualMethodInClass {
                                    class_name: name.clone(),
                                    method_name: mname.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                }
                // ⚠ 値としての**直接の自己参照フィールド**を禁止する（D37 / タスク 0-6）。
                //   `let next: Node` は無限階層構造になる（C の `struct S { S s; }` と同じ）。
                //   ⚠ **`Option[Node]` / `list[Node]` は許可**。インスタンスは実行時ポインタ
                //     （`Value::Instance(Rc<RefCell<InstanceData>>)`）なので無限にならず、
                //     連結リストや木が書ける。⇒ 型注釈が**クラス名そのもの**のときだけ弾く。
                //   ⚠ 相互再帰（`A.b: B` と `B.a: A`）は同じ理由で不正だが、クラスを跨ぐ
                //     循環検査が要るので**ここでは見ていない**（D37 は直接の自己参照のみ）。
                // ⚠⚠ メンバーに残った `!装飾子` はここで止める。型検査はフィールドを
                //   `Stmt::Field` として拾うので、包まれたままだと**そのメンバーだけ
                //   存在しないことになる**（`'C' has no member 'x'` という無関係な診断が出る）。
                // ⚠ エディタも展開してから型検査する（D5・タスク 5-0）ので、ここで未展開の装飾子に
                //   出会うのは本当に展開漏れのときだけ（以前はエディタだけ黙っていた）。展開に失敗した
                //   ときのエディタは型エラーを出さない（`MetaError` だけを出す）。
                for st in body {
                    if let Stmt::MetaDecorated { decorators, .. } = st {
                        let deco = match decorators.first() {
                            Some(crate::ast::Expr::Ident { name: d, .. }) => format!("!{d}"),
                            _ => "!...".to_string(),
                        };
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::UnexpandedMemberDecorator {
                                type_name: name.clone(),
                                decorator: deco,
                            },
                            span: None,
                        });
                    }
                }
                for st in body {
                    if let Stmt::Field { name: fname, type_ann, .. } = st {
                        if type_ann == name || type_ann == "Self" {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::SelfReferentialField {
                                    class_name: name.clone(),
                                    field_name: fname.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                }
                self.check_stmts(body);
                // 基底 trait の要求（フィールド型・メソッドシグネチャ）を満たすか（0-8）。
                // ⚠ 本体を検査した後に呼ぶ。クラスの型変数がまだ積まれている状態で
                //    見たいので `pop_type_params` より前に置く。
                self.check_trait_conformance(name);
                self.state.pop_type_params(saved_tp);
                self.state.exit_class(prev_class);
                self.state.exit_class_body();
                self.pop_scope();
            }
            Stmt::TraitDef { name, template_params, body, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                // ⚠⚠ **trait の型変数もスコープへ積む**（タスク 8.5 の計測で判明）。
                //    `ClassDef` は積んでいたのに `TraitDef` は積んでいなかったので、
                //    `trait Holder[T]: let item: T` の `T` が**スコープから見えない**まま
                //    本体を検査していた。`mentions_type_param` が偽を返すため、
                //    「存在しない型名」扱いになる経路がここだけ空いていた。
                let saved_tp =
                    self.state.push_type_params(template_params.iter().map(|p| p.name.clone()));
                self.push_scope();
                self.state.enter_class_body();
                self.check_stmts(body);
                self.state.exit_class_body();
                self.pop_scope();
                self.state.pop_type_params(saved_tp);
            }
            Stmt::ProtocolDef { name, .. } => {
                // プロトコルは型値としてスコープに登録する（インスタンス化試行を検出するため）
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::Protocol(name.clone(), Vec::new()))),
                    false,
                );
                // collect_fn_sigs で already registered in known_protocols
            }

            // --- ジャンプ文 ---
            Stmt::Return(expr, _) => {
                if let Some(e) = expr {
                    let got = self.infer(e);
                    // 不一致は戻り値の式を指す（3-3）。
                    self.with_anchor_at(e, |c| c.check_return_type(&got));
                }
                // ⚠ 値なしの `return`（早期脱出）は**照合しない**。「戻り値を返し忘れている」
                //    という別の検査であり、既存コードの早期 return を巻き込む。
            }
            Stmt::BlockReturn(expr, span) => {
                if self.state.block_return_forbidden() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::BlockReturnInLoopExpr,
                        span: Some(span.clone()),
                    });
                }
                // ⚠⚠ 値は囲みブロック式の `->T` 注釈と照合する（タスク 5.2・検体 X1/X2/X4）。
                //    以前は**実行時まで**判らなかった（`block ->int: block_return "s"` が
                //    通っていた）。
                let got = self.infer(expr);
                let expected = self.state.block_expr_expected().cloned();
                // 不一致は値の式を指す（3-3）。錨が無ければ文の位置（＝`block_return`）。
                self.with_anchor_at(expr, |c| c.check_block_expr_value(&got, expected, "block_return", None));
            }
            // ⚠ `loop_yield` は `for`/`while` 式のものでジェネレータとは別物。制限しない。
            Stmt::LoopYield(expr, _) => {
                // ⚠ 値は囲み式の `->list[T]` の**要素型**と照合する（タスク 5.2・検体 X3）。
                //
                // ⚠⚠ **いちばん内側の注釈が `list[T]` のときだけ**照合する。
                //    `block_return_typecheck.ar`（#35）が仕様として固定している:
                //    `for ... ->list[int]:` の中の `if ... ->int:` に書いた `loop_yield` は
                //    **検査されない**（内側の注釈が `list[T]` ではないから）。
                //    ここを「外側のループ式まで遡る」実装にすると偽エラーになる（実測）。
                let got = self.infer(expr);
                self.state.note_loop_yield(&got);
                let expected = match self.state.block_expr_expected() {
                    Some(InferredType::ListOf(elem)) => Some((**elem).clone()),
                    _ => None,
                };
                self.with_anchor_at(expr, |c| c.check_block_expr_value(&got, expected, "loop_yield", None));
            }
            Stmt::Yield(expr, _) => {
                // ⚠⚠ `yield` は **`gen` 本体の直下だけ**（bug_fix.md B13）。
                //    コルーチン化の前提（yield は自分のフレームにしか現れない）を守るため。
                if !self.state.in_gen_body() {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::YieldOutsideGenerator,
                        span: None,
                    });
                }
                // ⚠ 値は `gen` の宣言 yield 型と照合する（タスク 5.2・検体 C8）。
                //    照合先は `check_gen_def` が `current_fn_return` へ入れた要素型。
                let got = self.infer(expr);
                let expected = self.state.current_fn_return().cloned();
                self.with_anchor_at(expr, |c| c.check_block_expr_value(&got, expected, "yield", None));
            }

            // --- クラスフィールド宣言 ---
            Stmt::Field {
                name,
                kind,
                type_ann,
                default,
                ..
            } => {
                // ⚠ 素の容器型注釈は弾く（タスク 8.1・案 A）。
                self.check_ann_not_bare(type_ann, &format!("field `{name}`"));
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
                    // ⚠ 以前は `infer` の結果を**捨てていた**ので `const K: int = "s"` が
                    //    通っていた（0-2 のフィールド書き込みと同じ形の漏れ）。
                    let got = self.infer(expr);
                    let class_name = self
                        .state
                        .current_class()
                        .unwrap_or("<class>")
                        .to_string();
                    if !matches!(ty, InferredType::Unresolved | InferredType::Any)
                        && !self.mentions_type_param(&ty)
                    {
                        let ctx = format!("default value of field `{name}` of `{class_name}`");
                        // 不一致は既定値の式を指す（3-3）。
                        if !self.with_anchor_at(expr, |c| c.check_expected(&got, &ty, false, &ctx)) {
                            self.report_error_at(expr, StaticTypeError {
                                kind: TypeErrorKind::FieldTypeMismatch {
                                    field_name: name.clone(),
                                    class_name,
                                    expected: ty.clone(),
                                    got,
                                },
                                span: None,
                            });
                        }
                    }
                }
                // ⚠ メンバーは変数として**宣言しない**（name_scope_plan.md の 1-3・`CheckState::in_class_body`）。
                //   型・可変性はレジストリ（`class_field_details` / `field_is_mutable`）が持ち、`self.x` / `C.K` は
                //   そちらから引く。裸の名前で読むと `BareClassMember` になる（`check_name_defined`）。
            }

            // --- ジェネレータ関数定義 ---
            Stmt::GenDef {
                name,
                template_params,
                params,
                yield_type,
                body,
                ..
            } => self.check_gen_def(name, template_params, params, yield_type.as_deref(), body),

            // ⚠ メタ関数の**本体には触れない**（本体は展開時に走るので通常の型検査の対象外）。
            // ⚠⚠ ただし**純粋メタ関数は戻り値の型付きで宣言する**（タスク 1-6）。
            //   `code:` がメタ関数の外で書けなくなった（1-6）ので、`Code` が通常コードへ
            //   漏れる経路は「純粋メタ関数の返り値を束縛する」（`let f = frag()`）だけになった。
            //   `Unresolved` で宣言していると 1-5 の束縛点検査がそれを見逃す（実測）。
            //   ⚠ 展開しないエディタではこれが唯一の網。CLI では展開でメタ関数が消えるので、
            //     実行時の `NameError` に説明を添える側（3-8）が受け持つ。
            //   ⚠ 配置メタ関数は値を返さない（§1.1）ので従来どおり `Unresolved`。
            Stmt::MetaFnDef { name, return_type, is_placing, .. } => {
                let ty = if *is_placing {
                    InferredType::Unresolved
                } else {
                    InferredType::Function {
                        params: None,
                        return_type: Box::new(
                            return_type
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Unresolved),
                        ),
                        mutates_immutable: None,
                    }
                };
                self.declare(name.clone(), ty, false);
            }
            // `quote` の検査も 1-5。式は `Code` 型でなければならない。
            Stmt::Quote(_) => {}
            // ⚠ `!装飾子` は**包みなので透かして対象を検査する**。透かさないと
            //   装飾された宣言だけが型検査から抜け落ち、退行が緑のまま通る。
            //   装飾子式そのものの検査（メタ関数かどうか）は展開器（2-0）の仕事。
            Stmt::MetaDecorated { target, .. } => self.check_stmt(target),

            // --- new_type 定義 ---
            Stmt::NewTypeDef { name, .. } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
            }

            // --- enum 定義 ---
            Stmt::EnumDef { src: _, name, variants } => {
                self.declare(
                    name.clone(),
                    InferredType::TypeValOf(Box::new(InferredType::NamedInstance(name.clone()))),
                    false,
                );
                // ── 妥当性検査: バリアント値は `int`（タスク 3.4・検体 N2）─────────
                //
                // ⚠⚠ 実行時の `build_enum_classes` も同じ検査をするが、**定義が実行されない
                //    経路**では見逃していた。呼ばれない関数の中の
                //      fn never_called() -> int:
                //          enum Bad:
                //              A = "x"
                //    はプログラムが最後まで完走していた（実測）。
                // ⚠ これは整合性検査（3 分類）ではなく**妥当性検査**。「2 つの型が適合するか」
                //    ではなく「定義自身が成り立つか」を見る系統（D-14）。
                // ⚠ 推論できない値（`Unresolved`）は見送る。`a = g()` のような式値は
                //    呼び先の戻り値型が付くようになれば自然に検査される。
                for (vname, value) in variants.iter() {
                    let Some(expr) = value else { continue };
                    let got = self.infer(expr);
                    if matches!(got, InferredType::Unresolved | InferredType::Int) {
                        continue;
                    }
                    // 値の式を指す（3-4）。錨が無ければ文の位置（`enum`）。
                    self.report_error_at(expr, StaticTypeError {
                        kind: TypeErrorKind::EnumVariantNotInt {
                            enum_name: name.clone(),
                            variant: vname.clone(),
                            got: got.to_string(),
                        },
                        span: None,
                    });
                }
            }

            // --- 副作用のない文 ---
            Stmt::Pass
            | Stmt::Break
            | Stmt::Continue
            | Stmt::BreakPoint { .. }
            | Stmt::DebugLet(..) => {}

            // ⚠ `freeze x` は**再束縛だけ**を禁じる（タスク 7.2・検体 `Z7`）。
            //    `examples/basics/variable.ar` が
            //    `# temp = 0  # would be StaticTypeError after freeze` と仕様を明記している。
            // ⚠⚠ `freeze x` は **`mut` を `let` へ降格**する（タスク 9.2・利用者の決定）。
            //    再束縛だけでなく `x.append(..)` / `x[0] = ..` / `x.f = ..` も
            //    `let` と同じ規則で弾かれるようになる。
            Stmt::Freeze(name, _) => {
                // Python のオブジェクトは `let` へ降格できない（D-3・1-2）
                if let Some(ty) = self.lookup(name).map(|i| i.ty.clone()) {
                    self.require_mut_for_python_object(name, "freeze", &ty);
                }
                self.state.freeze_var(name)
            }

            // --- 例外処理 ---
            Stmt::Try {
                body,
                handlers,
                finally_body,
                span: _,
            } => {
                self.push_scope();
                self.check_stmts(body);
                self.pop_scope();
                for handler in handlers {
                    self.push_scope();
                    if let Some(name) = &handler.name {
                        // ⚠⚠ **捕捉した例外の型を付ける**（タスク 2.8）。以前は
                        //    `Unresolved` で宣言していたため、`Unresolved` が
                        //    `type_matches_exact` の万能受容体であることから
                        //      except ValueError as e:
                        //          let s: int = e.message   # str を int へ入れて通っていた
                        //    のように**束縛経由の全義務が無効化**されていた。
                        //    型は `exc_type` に書いてある（`None` は bare `except:`）。
                        // ⚠ bare `except:` は捕捉する型が判らないので `Unresolved` のまま
                        //    （取りこぼす方へ倒す）。
                        // ⚠ 修飾名（`except m.Err as e:`・タスク 4-3）はモジュールの修飾名へ引き直す
                        //   （`t.Tag` の注釈と同じ・10-8）。
                        let exc_ty = handler
                            .exc_type
                            .as_deref()
                            .map(|t| InferredType::NamedInstance(self.registry.resolve(t).into_owned()))
                            .unwrap_or(InferredType::Unresolved);
                        // Python で定義した例外（CPython の物）を受けるなら `except E as mut e` と書く（D-11・2-10）
                        if !handler.name_mut {
                            self.require_mut_for_python_object(name, "except", &exc_ty);
                        }
                        self.note_binding(name, &exc_ty);
                        self.declare(name.clone(), exc_ty, true);
                    }
                    // ⚠ 例外クラスでない型は**腕が永久に死ぬ**（タスク 7.1・検体 `E2`）。
                    //    `name` の有無に関係なく検査する。
                    if let Some(t) = handler.exc_type.as_deref() {
                        self.check_except_type(t);
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
            //
            // ⚠ 束縛は AST の `bind`（パーサが決める・CPython 準拠・2026-10-02）。`import a.b.c` は
            //   **パッケージ `a`** を束縛し、その型はパッケージのメンバーと子モジュールから組み立てる
            //   （`namespace_type`）。`bind: None` は束縛しない文（パッケージの連鎖を読み込むために
            //   パーサが足した文）で、メンバーの型を控えるだけ。
            Stmt::Import { lang, module, body, bind, origin, .. } => {
                self.annotate_module_body(lang, module, body);
                let member_types = self.module_member_types(lang, module, body);
                // ⚠ 読めなかった import（拡張だけ・`ImportOrigin::unresolved`）のモジュールは控えない。
                //   控えると「メンバーが 0 個のモジュール」として閉じ、使うたびに誤りを重ねる。
                if !origin.unresolved {
                    self.module_member_cache.insert(module.join("."), member_types);
                }
                if let Some(b) = bind {
                    let ns_ty = if origin.unresolved {
                        // 読めなかった import（拡張だけ）。誤りは拡張が診断として出している。
                        // `PyNamespace([])` を束縛すると未知メンバが `Any` になり、`d.Box.bump()` のような
                        // 連鎖アクセスが OperationOnAny エラーを重ねる（以前の拡張の
                        // examples/interop/py_decorators.ar で実際に発生した）。`Unresolved` は attribute
                        // access の match で `_ => {}` に落ちるため「型は分からないがエラーでもない」を表せる。
                        InferredType::Unresolved
                    } else {
                        self.namespace_type(lang, &b.module)
                    };
                    // 今の Arrow から呼べない C/C++ の関数（1-2）。cpp 系はパッケージではなく、束縛名が
                    // 別名かヘッダの名前なので、綴りは `束縛名.関数名`。
                    self.ffi_unsupported.retain(|k, _| !k.starts_with(&format!("{}.", b.name)));
                    // C/C++（1-2）と Rust（2-5）のスタブはモジュールの関数に印を付ける
                    if matches!(lang.as_str(), "cpp-dll" | "cpp-lib" | "rs") {
                        let depth = self.state.scope_depth();
                        let what = crate::type_check::ffi_kind(lang, false);
                        for (f, reason) in ffi_unsupported_fns(body) {
                            let spelled = format!("{}.{f}", b.name);
                            self.ffi_unsupported.insert(spelled.clone(), (depth, spelled, reason, what));
                        }
                    }
                    // `pd.read_csv(..)` で `@overload` の形を選ぶために、別名とモジュールを控える（1-3）
                    if lang == "py" {
                        self.py_aliases.insert(b.name.clone(), b.module.join("."));
                    }
                    self.declare(b.name.clone(), ns_ty, false);
                }
            }

            Stmt::FromImport { lang, module, names, body, origin, .. } => {
                self.annotate_module_body(lang, module, body);
                let member_types = self.module_member_types(lang, module, body);
                // 読めなかった import（拡張だけ）のモジュールは控えない（上の `Stmt::Import` と同じ理由）。
                if !origin.unresolved {
                    self.module_member_cache.insert(module.join("."), member_types.clone());
                }
                let marks_fns = matches!(lang.as_str(), "cpp-dll" | "cpp-lib" | "rs");
                let unsupported = if marks_fns { ffi_unsupported_fns(body) } else { Vec::new() };
                let what = crate::type_check::ffi_kind(lang, false);
                // CPython: 名前がモジュールに無ければサブモジュール（パーサが先に読み込んでいる）。
                let children = self.module_children.get(&module.join(".")).cloned().unwrap_or_default();
                // ⚠ Arrow のモジュールの名前にもサブモジュールにも無い名前は静的エラー（2026-10-02）。
                //   実行時は `ImportError: cannot import name ..`。
                let closed = if origin.unresolved { None } else { closed_module(lang, module) };
                for (orig_name, alias) in names {
                    let is_child = children.contains(orig_name);
                    if let Some(c) = &closed {
                        if !member_types.contains_key(orig_name.as_str()) && !is_child {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::CannotImportName {
                                    module: c.name.clone(),
                                    name: orig_name.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    let bind_name = alias.clone().unwrap_or_else(|| orig_name.clone());
                    // 今の Arrow から呼べない C/C++ の関数（1-2）。
                    if let Some((_, reason)) = unsupported.iter().find(|(n, _)| n == orig_name) {
                        self.ffi_unsupported.insert(
                            bind_name.clone(),
                            (self.state.scope_depth(), orig_name.clone(), reason.clone(), what),
                        );
                    } else {
                        self.ffi_unsupported.remove(&bind_name);
                    }
                    let child_path: Vec<String> =
                        module.iter().cloned().chain([orig_name.clone()]).collect();
                    let ty = member_types
                        .get(orig_name.as_str())
                        .cloned()
                        .or_else(|| is_child.then(|| self.namespace_type(lang, &child_path)))
                        .unwrap_or(
                            // 読めなかった import では `Any` に落とさない（上の Stmt::Import と同じ理由）。
                            if lang == "py" && !origin.unresolved {
                                InferredType::Any
                            } else {
                                InferredType::Unresolved
                            },
                        );
                    // ⚠ 取り込んだ名前は付け替えられない。**中身は元の宣言の属性に従う**
                    //   （`mut` のリストなら `X.append(..)` できる・`let` / `const` ならできない）。
                    let contents_mutable = self
                        .module_attr_cache
                        .get(&module.join("."))
                        .and_then(|a| a.get(orig_name.as_str()))
                        .is_some_and(|a| a.contents_mutable());
                    self.declare_imported(bind_name, ty, contents_mutable);
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
}


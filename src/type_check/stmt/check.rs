// stmt/check.rs — 文の静的型検査の中核: check_stmts / check_stmt。

use {
    crate::ast::{BinOp, Expr, FieldKind, MatchArm, MatchPattern, Param, Stmt, TupleTarget},
    crate::token::Span,
    crate::type_check::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind},
    crate::type_check::types::InferredType,
    crate::type_check::BinOperandKind,
    crate::type_check::TypeChecker,
};

/// この import 本体が「`editor` ビルドが読み込みを省略した結果の空」かどうか。
///
/// `editor` feature（VS Code 拡張の wasm ビルド）では `parser/imports_editor.rs` が
/// import 文を構文解釈だけして `body: vec![]` を返す。型検査側はその空を
/// 「メンバーが 0 個のモジュール」ではなく「**モジュールの中身が不明**」として扱う
/// 必要がある。両者を取り違えると、未知メンバーが `Any` に落ちて
/// エディタだけが偽陽性エラーを出す。
///
/// 通常ビルドでは常に `false` を返す（＝この分岐は消える）ので、
/// バイナリの型検査結果は一切変わらない。
#[inline]
fn editor_stub_body(body: &[Stmt]) -> bool {
    cfg!(feature = "editor") && body.is_empty()
}

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
                        if !self.check_expected(&rhs_ty, &declared, false, &ctx) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::VarTypeMismatch {
                                    name: name.clone(),
                                    expected: declared,
                                    got: rhs_ty,
                                },
                                span: Some(span.clone()),
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
            Stmt::AttrAssign { target, value } => {
                self.check_attr_assign(target, value, None);
            }
            Stmt::AttrCompoundAssign { target, op, value } => {
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
            } => self.check_if(branches, else_body),
            Stmt::Match { subject, arms, .. } => self.check_match(subject, arms),
            Stmt::While { cond, body } => {
                // ⚠ 条件は `bool` でなければならない（D-12・検体 K2）。
                self.walk_obligation_pending(cond, "5.5 条件は bool");
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
                let target_mut = self.path_is_mutable(iter).unwrap_or(false);
                for (t, ty) in targets.iter().zip(target_tys) {
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
                    if t != "_" && self.lookup(t).is_some() {
                        self.report_error(StaticTypeError {
                            kind: TypeErrorKind::VariableRedeclaration { name: t.clone() },
                            span: None,
                        });
                    }
                    self.declare(t.clone(), ty, target_mut);
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
                    self.check_decorator(dec, false, name);
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
                self.check_stmts(body);
                // 基底 trait の要求（フィールド型・メソッドシグネチャ）を満たすか（0-8）。
                // ⚠ 本体を検査した後に呼ぶ。クラスの型変数がまだ積まれている状態で
                //    見たいので `pop_type_params` より前に置く。
                self.check_trait_conformance(name);
                self.state.pop_type_params(saved_tp);
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
                    let got = self.infer(e);
                    self.check_return_type(&got);
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
                self.check_block_expr_value(&got, expected, "block_return", Some(span.clone()));
            }
            // ⚠ `loop_yield` は `for`/`while` 式のものでジェネレータとは別物。制限しない。
            Stmt::LoopYield(expr) => {
                // ⚠ 値は囲み式の `->list[T]` の**要素型**と照合する（タスク 5.2・検体 X3）。
                //
                // ⚠⚠ **いちばん内側の注釈が `list[T]` のときだけ**照合する。
                //    `block_return_typecheck.ar`（#35）が仕様として固定している:
                //    `for ... ->list[int]:` の中の `if ... ->int:` に書いた `loop_yield` は
                //    **検査されない**（内側の注釈が `list[T]` ではないから）。
                //    ここを「外側のループ式まで遡る」実装にすると偽エラーになる（実測）。
                let got = self.infer(expr);
                let expected = match self.state.block_expr_expected() {
                    Some(InferredType::ListOf(elem)) => Some((**elem).clone()),
                    _ => None,
                };
                self.check_block_expr_value(&got, expected, "loop_yield", None);
            }
            Stmt::Yield(expr) => {
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
                self.check_block_expr_value(
                    &got,
                    self.state.current_fn_return().cloned(),
                    "yield",
                    None,
                );
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
                        if !self.check_expected(&got, &ty, false, &ctx) {
                            self.report_error(StaticTypeError {
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
                // ⚠ `StaticMut` も可変（タスク 1.2）。理由は
                //    `registry/builder.rs` の同じ式のコメントを参照。2 箇所あるので
                //    **片方だけ直すとずれる**。
                let mutable = matches!(kind, FieldKind::Mut | FieldKind::StaticMut);
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
            Stmt::EnumDef { name, variants } => {
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
                    self.report_error(StaticTypeError {
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
                        // ⚠⚠ **捕捉した例外の型を付ける**（タスク 2.8）。以前は
                        //    `Unresolved` で宣言していたため、`Unresolved` が
                        //    `type_matches_exact` の万能受容体であることから
                        //      except ValueError as e:
                        //          let s: int = e.message   # str を int へ入れて通っていた
                        //    のように**束縛経由の全義務が無効化**されていた。
                        //    型は `exc_type` に書いてある（`None` は bare `except:`）。
                        // ⚠ bare `except:` は捕捉する型が判らないので `Unresolved` のまま
                        //    （取りこぼす方へ倒す）。
                        let exc_ty = handler
                            .exc_type
                            .as_deref()
                            .map(|t| InferredType::NamedInstance(t.to_string()))
                            .unwrap_or(InferredType::Unresolved);
                        self.declare(name.clone(), exc_ty, true);
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
                let ns_ty = if editor_stub_body(body) {
                    // `editor`（VS Code 拡張の wasm ビルド）は import 先を読み込まないので
                    // body が空になる。ここで `PyNamespace([])` を束縛すると未知メンバが
                    // `Any` になり、`d.Box.bump()` のような連鎖アクセスが
                    // OperationOnAny エラー＝**エディタだけが出す偽陽性**になる
                    // （examples/interop/py_decorators.ar で実際に発生した）。
                    // `Unresolved` は attribute access の match で `_ => {}` に落ちるため
                    // 「型は分からないがエラーでもない」を正しく表現できる。
                    InferredType::Unresolved
                } else if lang == "py" || lang == "py-int" {
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
                    let ty = member_types.get(orig_name.as_str()).cloned().unwrap_or(
                        // `editor` の空 body では `Any` に落とさない（上の Stmt::Import と同じ理由）。
                        if is_py && !editor_stub_body(body) {
                            InferredType::Any
                        } else {
                            InferredType::Unresolved
                        },
                    );
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
        self.check_match_arms(subject, arms);
    }

    /// 型ガードの型名が存在するかを検査する（**妥当性検査**・タスク 3.4・検体 X6）。
    ///
    /// ⚠⚠ これが無いと `is NoSuchType:` が**黙って通り、腕が永久に死ぬ**。さらに腕の中では
    /// 対象変数がその存在しないクラスへ絞り込まれるので、**メンバーアクセスが全て無検査**に
    /// なる（`d.totally_missing_field` が通っていた・実測）。
    ///
    /// ⚠ 整合性検査（3 分類）ではなく**妥当性検査**。「2 つの型が適合するか」ではなく
    /// 「名前が在るか」を見る別系統（D-14）。
    /// ⚠ **判らない名前は通さない**が、型変数（`fn f[T]` の `T`）は正当なので除く。
    pub(crate) fn check_guard_type_exists(&mut self, type_name: &str) {
        // プリミティブ・`Any` 等は `from_ann` が解釈できるので、それで判定する。
        // ⚠ `from_ann` は**大文字始まりの未知の識別子をクラス名にする**ので、
        //    `Some(..)` でも「在る」ことの証明にはならない（`NamedInstance` は要確認）。
        match InferredType::from_ann(type_name) {
            Some(InferredType::NamedInstance(n)) => {
                if self.state.is_type_param(n.as_str()) {
                    return; // テンプレート型変数は正当
                }
                if self.registry.is_known_class(n.as_str())
                    || self.registry.is_protocol(n.as_str())
                    || self.registry.is_known_trait(n.as_str())
                {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                    span: None,
                });
            }
            // プリミティブ・コレクション・`Union` 等は解釈できた時点で存在が確かめられている。
            Some(_) => {}
            // `from_ann` が解釈できない綴りは未知として扱う。
            None => self.report_error(StaticTypeError {
                kind: TypeErrorKind::UnknownGuardType { type_name: type_name.to_string() },
                span: None,
            }),
        }
    }

    /// `match` の腕を型検査する（**文と式で共有**・タスク 2.9）。
    ///
    /// ⚠⚠ **以前は `match` 式がこの絞り込みを持っていなかった。** `Expr::MatchExpr`
    /// （`infer.rs`）は `Case` の infer と腕本体の検査だけで `IsType` の絞り込みを
    /// していなかったため、**文と式で意味論が違った**（実測）:
    ///
    /// ```arrow
    /// match v:                     # 文 → 絞り込まれる
    ///     is Dog:
    ///         let s: str = v.n     # ✅ int → str でエラー
    ///
    /// let r = match v ->int:       # 式 → 絞り込まれなかった
    ///     is Dog:
    ///         let s: str = v.n     # ⛔ 通っていた
    /// ```
    ///
    /// ⇒ 腕の処理を 1 箇所に集約し、文・式の両方から呼ぶ。**別々に書くと再びずれる。**
    pub(crate) fn check_match_arms(&mut self, subject: &Expr, arms: &[MatchArm]) {
        let _subject_ty = self.infer(subject);
        // 絞り込めるのは対象が**単なる識別子**のときだけ（再 `declare` で実装しているため）。
        let subject_name: Option<String> = if let Expr::Ident { name: n, .. } = subject {
            Some(n.clone())
        } else {
            None
        };
        for arm in arms {
            self.push_scope();
            match &arm.pattern {
                MatchPattern::Case(expr) => {
                    // ⚠ パターンの型は subject の型と一致しなければならない（検体 X5）。
                    self.walk_obligation_pending(expr, "5.4 case パターン型 vs subject");
                }
                MatchPattern::IsType(type_name) => {
                    self.check_guard_type_exists(type_name);
                    if let Some(ref var_name) = subject_name {
                        let narrowed = Self::type_from_guard_name(type_name);
                        let is_mut = self.lookup(var_name).map(|v| v.mutable).unwrap_or(false);
                        self.declare(var_name.clone(), narrowed, is_mut);
                    }
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

            // ⚠ 条件は `bool` でなければならない（D-12・検体 K1）。
            self.walk_obligation_pending(cond, "5.5 条件は bool");

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
    ///
    /// `compound_op` が `None` のとき（＝通常代入）、フィールドの**宣言型**と
    /// 代入値の型を突き合わせる（0-2）。`Some(op)` のとき（＝複合代入）は
    /// [`TypeChecker::check_compound_assign`] の二段検査へ回す（D-8・タスク 5.1）。
    fn check_attr_assign(&mut self, target: &Expr, value: &Expr, compound_op: Option<&BinOp>) {
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
        // ⚠ `infer(target)` は属性なら**フィールドの宣言型**を返す（`infer_attr` が
        //    `class_field_details` から引く）。この戻り値をそのまま期待型に使うことで、
        //    クラス名の解決のために**オブジェクトを二度推論しないで済む**。
        //    二度推論すると `infer_attr` がオブジェクトに対して出す診断
        //    （`OperationOnAny` など）が重複する。
        let target_ty = self.infer(target);
        let value_ty = self.infer(value);
        let Expr::Attr { object, attr, span, .. } = target else {
            return;
        };
        // レシーバのクラス名。**推論を伴わない**スコープ引きで求める（`self` も
        // `NamedInstance(現在のクラス)` として束縛されているので同じ経路で引ける）。
        // 識別子以外のレシーバ（`f().x = v` など）は保守的に検査しない。
        // ⚠ `GenericInstance{Box,[int]}` も取りこぼさないこと（A-2）。`NamedInstance` だけを
        //    見ていると、テンプレートクラスのインスタンスに対する検査が**黙って消える**。
        let (class_name, subst) = match object.as_ref() {
            Expr::Ident { name, .. } => {
                let ty = self.lookup(name).map(|i| i.ty.clone());
                match ty.as_ref().and_then(|t| self.class_and_subst(t)) {
                    Some(pair) => pair,
                    None => return,
                }
            }
            _ => return,
        };
        // 期待型を決める。
        //
        // ⚠⚠ `infer_attr` が引く `registry.class_field_details` は**そのクラス自身が
        //    宣言したフィールドしか持たない**。trait から継承したフィールド（`w.hp`）は
        //    **別のテーブル `trait_field_details`** に入っているので `Unresolved` になり、
        //    検査が素通りしていた（起票されたバグ報告が trait を挙げていたのはこの形）。
        //    ⚠ `collect_class_field_details` も `class_field_details` しか見ないので
        //      これだけでは足りない。基底 trait のテーブルを明示的に引く。
        let expected = match target_ty {
            InferredType::Unresolved | InferredType::Any => {
                // ⚠ ここも**置換表を通す**（A-2）。`declared_field_type` はレジストリから
                //   置換前の型を返すので、通さないと型変数（`T`）がそのまま比較されて
                //   `declared 'T'` という偽陽性になる（実測）。
                match self
                    .declared_field_type(&class_name, attr)
                    .map(|t| Self::subst_type_params(&t, &subst))
                {
                    Some(ty) => ty,
                    None => return, // フィールドでない（メソッド名など）
                }
            }
            t => t,
        };
        // 宣言型が判らないフィールド（外部言語オブジェクト・型変数）は検査しない。
        // ⚠ ここを外すと `type_matches(値, Unresolved)` が false になり**偽陽性が出る**。
        if matches!(expected, InferredType::Unresolved | InferredType::Any) {
            return;
        }
        // ⚠ テンプレートクラスの `mut v: T` は具体型と突き合わせられない
        //    （`mentions_type_param` の doc）。
        if self.mentions_type_param(&expected) {
            return;
        }
        // ⚠⚠ **複合代入はここで分岐する**（D-8・タスク 5.1）。格納されるのは `value` ではなく
        //    `field <op> value` の結果なので、`value` をフィールド型と突き合わせるのは**嘘**。
        //    二段検査へ回す（以前はこの地点の検査を丸ごと省いていた ＝ 検体 `F2`）。
        if let Some(op) = compound_op {
            let target_desc = format!("{}.{}", Self::attr_object_desc(object), attr);
            self.check_compound_assign(&target_desc, op, &expected, &value_ty, Some(span.clone()));
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("field `{attr}` of class `{class_name}`");
        if self.check_expected(&value_ty, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::FieldTypeMismatch {
                field_name: attr.clone(),
                class_name,
                expected,
                got: value_ty,
            },
            span: Some(span.clone()),
        });
    }

    /// 属性代入のレシーバをエラー文言用の綴りにする（`c.n` の `c` の部分）。
    ///
    /// ⚠ 識別子以外のレシーバ（`f().x` など）はここへ来ない（呼び出し元が
    /// `Expr::Ident` でなければ先に `return` する）が、将来広げたときに
    /// 文言が壊れないよう既定値を置く。
    fn attr_object_desc(object: &Expr) -> String {
        match object {
            Expr::Ident { name, .. } => name.clone(),
            _ => "<expr>".to_string(),
        }
    }

    /// 仮引数の**既定値**の型を、その仮引数の宣言型と突き合わせる（0-12）。
    ///
    /// ⚠⚠ 既定値の式は**推論すらされていなかった**。`check_fn_def` は注釈の有無しか見ず
    /// `param.default` に触れず、`declare_param` も無視していたため
    /// `fn f(let n: int = "wrong")` が静的にも実行時にも通っていた（実測）。
    ///
    /// ⚠ 可変長パラメータは既定値を持てない（パーサが弾く）ので対象外。
    fn check_param_defaults(&mut self, func_name: &str, params: &[Param]) {
        for p in params {
            let Some(expr) = &p.default else { continue };
            // ⚠ 推論は**必ず行う**（注釈が無くても）。注釈テーブルへ焼くため。
            let got = self.infer(expr);
            let Some(declared) = p.type_ann.as_deref().and_then(InferredType::from_ann) else {
                continue; // 注釈なし／解釈できない綴り（欠落は別途 `MissingParamTypeAnn`）
            };
            if matches!(declared, InferredType::Unresolved | InferredType::Any)
                || self.mentions_type_param(&declared)
            {
                continue;
            }
            let ctx = format!("default value of `{}` of `{func_name}`", p.name);
            // ⚠ `mut` 引数でも既定値は**呼び元の記憶域ではない**ので拡大を許してよい
            //    （write-back の相手が居ない）。
            if self.check_expected(&got, &declared, false, &ctx) {
                continue;
            }
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ParamDefaultTypeMismatch {
                    func_name: func_name.to_string(),
                    param_name: p.name.clone(),
                    expected: declared,
                    got,
                },
                span: None,
            });
        }
    }

    /// この型が**テンプレート型変数を含む**か（`T` / `list[T]` / `dict[str, T]` …）。
    ///
    /// ⚠⚠ 含むなら**検査しない**。`from_ann` は大文字始まりの未知の識別子を
    /// `NamedInstance` にするので、型変数と実在のクラスが型の上では区別できない。
    /// 具体型と突き合わせると `class Box[T]: mut v: T` の `self.v = 0` が
    /// 「`T` に `int` を入れた」という**偽のエラー**になる（実測）。
    ///
    /// 実体化後の AST は型変数が具体型へ置換済みなので、検査したい形はそちらで見られる。
    pub(crate) fn mentions_type_param(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(n) => self.state.is_type_param(n),
            InferredType::GenericInstance { args, .. } => {
                args.iter().any(|a| self.mentions_type_param(a))
            }
            InferredType::ListOf(t)
            | InferredType::FixedListOf(t)
            | InferredType::ListLikeOf(t)
            | InferredType::SetOf(t)
            | InferredType::TypeValOf(t) => self.mentions_type_param(t),
            InferredType::DictOf(k, v) => {
                self.mentions_type_param(k) || self.mentions_type_param(v)
            }
            InferredType::Result(a, b) => {
                self.mentions_type_param(a) || self.mentions_type_param(b)
            }
            InferredType::Union(ts) | InferredType::Intersection(ts) | InferredType::Tuple(ts) => {
                ts.iter().any(|t| self.mentions_type_param(t))
            }
            _ => false,
        }
    }

    /// 型注釈中の `Self` を現在のクラスへ解決する（クラス外ならそのまま）。
    fn resolve_self_type(&self, ty: InferredType) -> InferredType {
        match (&ty, self.state.current_class()) {
            (InferredType::SelfType, Some(cls)) => InferredType::NamedInstance(cls.to_string()),
            _ => ty,
        }
    }

    /// `return <値>` の型を、宣言された戻り値型と突き合わせる（0-3）。
    ///
    /// ⚠ 宣言が無い / 解決できない関数では何もしない。戻り値注釈そのものが無い場合は
    /// `MissingReturnTypeAnn` が別途出るので、ここで二重に鳴らさない。
    fn check_return_type(&mut self, got: &InferredType) {
        let Some(expected) = self.state.current_fn_return().cloned() else {
            return;
        };
        if matches!(expected, InferredType::Unresolved | InferredType::Any) {
            return;
        }
        // ⚠ テンプレート型変数を含む戻り値型は検査しない（`mentions_type_param` の doc）。
        if self.mentions_type_param(&expected) {
            return;
        }
        let func_name = self.state.current_fn().unwrap_or("<fn>").to_string();
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("return value of `{func_name}`");
        if self.check_expected(got, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::ReturnTypeMismatch {
                func_name,
                expected,
                got: got.clone(),
            },
            span: None,
        });
    }

    /// `block_return` / `loop_yield` / `yield` の値を、囲み構文が宣言した型と照合する
    /// （タスク 5.2・検体 `X1`〜`X4` / `C8`）。
    ///
    /// ⚠⚠ **3 つとも「値を囲み構文へ渡す」同じ義務**なので 1 本にした。地点ごとに
    /// 書き分けると、`block_return` だけ直して `loop_yield` が漏れる形になる
    /// （実際に 3 地点とも別々に無検査だった）。
    ///
    /// `expected` が `None` のとき（注釈なし・解決できない注釈・囲みが無い）は照合しない。
    fn check_block_expr_value(
        &mut self,
        got: &InferredType,
        expected: Option<InferredType>,
        keyword: &'static str,
        span: Option<Span>,
    ) {
        let Some(expected) = expected else { return };
        // ⚠ テンプレート型変数を含む型は具体型と突き合わせられない（`mentions_type_param`）。
        if self.mentions_type_param(&expected) {
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("value of `{keyword}`");
        if self.check_expected(got, &expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::BlockExprValueMismatch {
                keyword: keyword.to_string(),
                expected,
                got: got.clone(),
            },
            span,
        });
    }

    /// クラス `class_name` のフィールド `field` の**宣言型**を引く。
    ///
    /// ⚠⚠ フィールドの宣言は**2 つのテーブルに分かれて**入っている:
    /// - 自クラスと基底クラスが宣言したもの … `class_field_details`
    /// - **基底 trait** が宣言したもの        … `trait_field_details`
    ///
    /// 前者しか見ないと `class Wolf(Creature)` の `w.hp`（`hp` は `Creature` 由来）が
    /// 引けず、型検査が素通りする。`build_field_index` が実行時に
    /// 「trait のフィールドを先頭に、own を後ろに」と**1 つのスロット列へ畳んでいる**のと
    /// 同じものを、静的側でも 1 つに見せるための関数。
    fn declared_field_type(&self, class_name: &str, field: &str) -> Option<InferredType> {
        // own（＋クラス継承）を先に見る。own の宣言が trait の宣言を上書きするため。
        if let Some((_, ty)) = self.collect_class_field_details(class_name).get(field) {
            return Some(ty.clone());
        }
        // 基底 trait を宣言順に辿る。複数 trait が同名フィールドを持つ場合は
        // 実行時も同一スロットを共有する（`build_field_index`）ので、最初の 1 件でよい。
        for base in self.registry.class_bases(class_name).unwrap_or(&[]) {
            if let Some((_, ty)) = self
                .registry
                .trait_field_details(base)
                .and_then(|m| m.get(field))
            {
                return Some(ty.clone());
            }
        }
        None
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
        template_params: &[crate::ast::TemplateParam],
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
        // ⚠⚠ **関数名は関数値としての型で宣言する**（タスク 2.2）。
        //    以前は `Unresolved` で宣言していたため、`fn` を名前で参照した値の型が
        //    `Unresolved` になり、`type_matches_exact` の万能受容体として
        //      let x: int = wrong                       # int 変数に関数が入る
        //      let f: function[int]->int = takes_str    # シグネチャ違いが通る
        //    が黙って通っていた。引数・戻り値は `fn_sigs` に揃っている。
        //    ⚠ `fn_value_type` が `None` を返す場合（オーバーロード・テンプレート関数）は
        //    従来どおり `Unresolved`。関数値としての型が 1 つに決まらないため。
        let self_ty = self.fn_value_type(name).unwrap_or(InferredType::Unresolved);
        self.declare(name.to_string(), self_ty, false);
        self.push_scope();
        // ⚠ 関数自身の型変数（`fn f[T]`）を積む。囲みクラスの型変数は `ClassDef` 側が
        //    既に積んでいるので、ここでは追加するだけでよい。
        let saved_tp =
            self.state.push_type_params(template_params.iter().map(|p| p.name.clone()));
        // ⚠ **既定値の検査は仮引数を宣言する前**に行う。既定値は他の仮引数を参照できない
        //    （参照できてしまうと評価順に依存する）ので、まだ見えていない状態で推論する。
        // ⚠ 型変数は既に積んであるので `mentions_type_param` が効く（`fn g[T](let n: T = …)`）。
        self.check_param_defaults(name, params);
        for param in params {
            self.declare_param(param);
        }
        // ⚠ 戻り値型は**注釈から**取る（0-3）。`current_fn_name` でレジストリを引くと
        //    オーバーロード・メソッド・入れ子関数で一意に定まらない。
        //    `Self` は現在のクラスへ解決しておく（`return Self(...)` を通すため）。
        let declared_ret = return_type
            .and_then(InferredType::from_ann)
            .map(|t| self.resolve_self_type(t));
        let prev_fn = self.state.enter_fn(name.to_string(), declared_ret);
        // ⚠⚠ **継承しない**。`gen` の中の入れ子 `fn` もここを通るので、
        //    false へ張り替えることでそこの `yield` もエラーになる。
        let prev_gen = self.state.enter_gen_body(false);
        // ⚠ ブロック式の照合先も**継承しない**（`with_fn_body` の doc・タスク 5.2）。
        self.with_fn_body(|c| c.check_stmts(body));
        self.state.exit_gen_body(prev_gen);
        self.state.exit_fn(prev_fn);
        self.state.pop_type_params(saved_tp);
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
        // `gen` 本体の直下だけが `yield` を書ける（B13）。
        let prev_gen = self.state.enter_gen_body(true);
        // ⚠ `gen` の `->T` は**要素型**（`yield` 1 回分の型）。`check_return_type` が
        //    使う `current_fn_return` と同じ場所へ入れて `yield` の照合に使う（タスク 5.2）。
        let prev_fn = self.state.enter_fn(
            name.to_string(),
            yield_type.and_then(InferredType::from_ann),
        );
        self.with_fn_body(|c| c.check_stmts(body));
        self.state.exit_fn(prev_fn);
        self.state.exit_gen_body(prev_gen);
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

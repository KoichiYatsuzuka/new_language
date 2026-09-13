use crate::ast::{Expr, UnaryOp};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// 部分木を歩いて診断を出すだけ。**この地点に型義務は無い**（タスク 3.1）。
    ///
    /// ⚠⚠ `infer` は「型を返す」と「部分木を歩いて診断を出す」を兼ねていたため、
    /// `self.infer(e);` と結果を捨てる書き方が**正当な場合と義務忘れの場合で区別が
    /// 付かなかった**。0-1〜A-4 で塞いだ 16 件の穴はすべてこの形だった。
    /// ⇒ `infer` に `#[must_use]` を付け、捨てる意図を**この 2 つのラッパで明示**する。
    #[inline]
    pub(super) fn walk(&mut self, expr: &Expr) {
        let _ = self.infer(expr);
    }

    /// 部分木を歩くが、**この地点には型義務があり未実装**（タスク 3.1）。
    ///
    /// `task` に実装タスク番号を書く。`walk` と分けてあるのは
    /// **「義務が無い」と「義務を忘れている」を取り違えないため**で、
    /// `grep walk_obligation_pending` で未実装の義務地点を数え上げられる。
    /// ⚠ 外側の網は `scripts/type_obligations.ps1`（114 件の検体）。
    /// こちらはコード側の記録で、2 つは独立した網。
    #[inline]
    pub(super) fn walk_obligation_pending(&mut self, expr: &Expr, _task: &'static str) {
        let _ = self.infer(expr);
    }

    /// 式の型を推論して [`InferredType`] を返す。副作用として型エラーを収集する場合がある。
    ///
    /// ⚠⚠ **`#[must_use]`**（タスク 3.1）。結果を捨てたいときは [`Self::walk`] か
    /// [`Self::walk_obligation_pending`] を使うこと。直接 `self.infer(e);` と書くと
    /// 警告になる ＝ 「義務を忘れた」のか「歩くだけ」なのかを宣言させる仕掛け。
    #[must_use]
    pub(super) fn infer(&mut self, expr: &Expr) -> InferredType {
        match expr {
            // --- リテラル ---
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::ImaginaryLit(_) => InferredType::Complex,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
            Expr::Undefined => InferredType::Undefined,
            // ⚠⚠ 要素型は**合成する**（タスク 2.7・原因②の修正）。以前は
            //    「全要素が同型でなければ素の `List` に落とす」実装だったが、素の `List` は
            //    `is_list_like` 規則で `ListOf(任意)` と適合するため、
            //    **捨てることが「何でも通す」になっていた**:
            //      let xs: list[int]   = [1, "s"]   # 通っていた
            //      let xs: list[str]   = [1, "s"]   # 通っていた
            //      let xs: list[float] = [1, "s"]   # 通っていた
            //    ⇒ 混在は `Union` へ合成する（`join_elem_types`）。
            Expr::List(elems) => {
                if elems.is_empty() {
                    // ⚠⚠ 空リテラルの要素型は **`Never`（⊥）**（D-11・タスク 4.1）。
                    //    `Never` はあらゆる型へアップキャストできるので
                    //    `mut xs: list[int] = []` が通る。
                    //    ⚠ `Any`（上端）にしてはいけない — `list[Any]` は `list[int]` の
                    //    スーパータイプなので、D-3（アップキャストのみ）のもとでは
                    //    `list[Any]` → `list[int]` がダウンキャストになって落ちる。
                    InferredType::ListOf(Box::new(InferredType::Never))
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    InferredType::ListOf(Box::new(Self::join_elem_types(types)))
                }
            }
            Expr::Set(elems) => {
                if elems.is_empty() {
                    InferredType::SetOf(Box::new(InferredType::Never))
                } else {
                    let types: Vec<InferredType> = elems.iter().map(|e| self.infer(e)).collect();
                    InferredType::SetOf(Box::new(Self::join_elem_types(types)))
                }
            }
            Expr::Tuple(exprs) => {
                let types: Vec<InferredType> = exprs.iter().map(|e| self.infer(e)).collect();
                InferredType::Tuple(types)
            }

            // --- 属性アクセス ---
            Expr::Attr { object, attr, span, node_id, .. } => {
                self.infer_attr(object, attr, span, *node_id)
            }
            Expr::TraitAccess { object, .. } => {
                self.walk(object);
                InferredType::Unresolved
            }

            // --- 関数呼び出し ---
            Expr::Call { func, args, node_id, .. } => {
                let result = self.infer_call(func, args, *node_id);
                // ── AST 型解決層（#16）── 呼び出しの**結果型**を焼く（CallInfo は infer_call_inner が充填）。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }

            // --- 識別子 ---
            // 記憶域の解決（`res`）はリゾルバが型検査の後に書くので、ここでは常に未解決。
            // 型検査は名前でスコープを引くだけで `res` を見ない。
            Expr::Ident { name, node_id, .. } => {
                let result = self
                    .lookup(name)
                    .map(|v| v.ty.clone())
                    // ⚠⚠ **変数スコープに無ければ関数を探す**（タスク 2.2）。
                    //    以前はここで `Unresolved` に倒していたため、`fn` を名前で参照した
                    //    **関数値に型が付いていなかった**。`Unresolved` は
                    //    `type_matches_exact` の万能受容体なので
                    //      let x: int = wrong                       # int 変数に関数が入る
                    //      let f: function[int]->int = takes_str    # シグネチャ違いが通る
                    //    が黙って通っていた。引数・戻り値は `fn_sigs` に揃っている。
                    .or_else(|| self.fn_value_type(name.as_str()))
                    .unwrap_or(InferredType::Unresolved);
                // ── AST 型解決層（#15b）── 参照サイトごとの型を焼く。
                // 変数単位ではなく**参照位置単位**なのが要点で、型ガード絞り込みは
                // 分岐スコープでの再 `declare` として実装されているため、同じ変数でも
                // 参照位置によって `lookup` の答えが変わる。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }

            // --- local::name 変数 ---
            Expr::LocalVar(name) => {
                let key = format!("local::{}", name);
                self.lookup(&key)
                    .map(|v| v.ty.clone())
                    .unwrap_or(InferredType::Unresolved)
            }

            // --- 単項演算子 ---
            Expr::UnaryOp { op, operand } => self.infer_unaryop(op, operand),

            // --- 二項演算子 ---
            Expr::BinOp {
                op,
                left,
                right,
                span,
                node_id,
            } => {
                let lt = self.infer(left);
                let rt = self.infer(right);
                self.check_binop(op, &lt, &rt, span.clone());
                let result = Self::infer_binop_result(op, &lt, &rt);
                // ── AST 型解決層（#16）── 二項演算の**結果型**を焼く。
                self.annotations.set_resolved(*node_id, result.clone());
                // 両オペランドが同一プリミティブ（int/int・float/float）なら**オペランド種別**も焼く
                // （plan A: VM が型特化 op でタグ検査・op ディスパッチを省く判断に使う）。
                // 判断は `BinOperandKind::of` に集約（`CompoundAssign` と共有）。
                let kind = super::annotations::BinOperandKind::of(&lt, &rt);
                match kind {
                    Some(k) => self.annotations.set_binop_kind(*node_id, k),
                    // 特化できなかった理由を数える（#16 段階 D の診断）。
                    None => {
                        let lu = matches!(lt, InferredType::Unresolved);
                        let ru = matches!(rt, InferredType::Unresolved);
                        self.annotations.note_binop_miss(lu, ru);
                        // `Unresolved` を生んだ式の種類を記録する（どこを直すと効くかの特定用）。
                        if lu {
                            self.annotations.note_unresolved_source(expr_kind_name(left));
                        }
                        if ru {
                            self.annotations.note_unresolved_source(expr_kind_name(right));
                        }
                    }
                }
                result
            }

            // --- テンプレート実体化 ---
            // `Box[int]` を**呼び出さずに**値として使う形（`let c = Box[int]` 等）。
            // ⚠ 呼び出し `Box[int](..)` の結果型は `template_call_result_type`（タスク 2.1）。
            //    こちらは**型値**なので、素のクラス名 `C` が `TypeValOf(NamedInstance("C"))`
            //    になるのと揃えて `TypeValOf(GenericInstance{..})` を返す。
            Expr::TemplateInstantiate { base, type_args } => {
                self.walk(base);
                let Expr::Ident { name, .. } = base.as_ref() else {
                    return InferredType::Unresolved;
                };
                if !self.registry.is_known_class(name.as_str()) {
                    // テンプレート関数の型値。シグネチャの決定は呼び出し点に任せる。
                    return InferredType::Unresolved;
                }
                let Some(tparams) = self.registry.template_params(name.as_str()) else {
                    return InferredType::Unresolved;
                };
                if tparams.len() != type_args.len() {
                    return InferredType::Unresolved;
                }
                let mut args = Vec::with_capacity(type_args.len());
                for a in type_args {
                    match InferredType::from_ann(a) {
                        Some(t) if !matches!(t, InferredType::Unresolved) => args.push(t),
                        _ => return InferredType::Unresolved,
                    }
                }
                InferredType::TypeValOf(Box::new(InferredType::GenericInstance {
                    name: name.clone(),
                    args,
                }))
            }

            // --- 辞書・サブスクリプト ---
            Expr::Dict(pairs) => {
                if pairs.is_empty() {
                    InferredType::DictOf(
                        Box::new(InferredType::Never),
                        Box::new(InferredType::Never),
                    )
                } else {
                    let key_types: Vec<InferredType> =
                        pairs.iter().map(|(k, _)| self.infer(k)).collect();
                    let val_types: Vec<InferredType> =
                        pairs.iter().map(|(_, v)| self.infer(v)).collect();
                    // ⚠ キー・値それぞれを合成する（タスク 2.7）。以前はどちらかが
                    //    揃わないだけで素の `Dict` に落ち、`dict[任意, 任意]` と適合していた。
                    InferredType::DictOf(
                        Box::new(Self::join_elem_types(key_types)),
                        Box::new(Self::join_elem_types(val_types)),
                    )
                }
            }
            Expr::Subscript { object, index, node_id } => {
                let obj_ty = self.infer(object);
                let idx_ty = self.infer(index);
                // ⚠ 添字の型を検査する（タスク 5.2b・検体 L6 / L7）。
                self.check_subscript_index(&obj_ty, &idx_ty);
                // ⚠⚠ **スライス添字は要素ではなく「同じ種類の容器」を返す**（タスク 5.2b）。
                //    以前は添字がスライスでも要素型を返していたので
                //    `let y: int = xs[0:2]` が**黙って通り**、実行時は `[1, 2]` が入っていた。
                //    実測（`eval_subscript_slice`）: list→list ／ str→str ／ tuple→tuple ／
                //    クラスは `__getitem__` へ委譲 ／ それ以外は実行時 TypeError。
                if Self::is_slice_type(&idx_ty) {
                    let result = match &obj_ty {
                        InferredType::List | InferredType::ListOf(_) | InferredType::Str => {
                            obj_ty.clone()
                        }
                        // ⚠ tuple は**要素数が変わる**ので静的には決められない。
                        // ⚠ fixed_list / list_like / set / dict はスライスできない
                        //    （実行時 TypeError）。診断は出さず判定不能に倒す。
                        _ => InferredType::Unresolved,
                    };
                    self.annotations.set_resolved(*node_id, result.clone());
                    return result;
                }
                let result = match obj_ty {
                    InferredType::ListOf(elem) | InferredType::FixedListOf(elem) | InferredType::ListLikeOf(elem) => *elem,
                    // ⚠⚠ `set` は**添字アクセスできない**（実測: `'set' object is not
                    //    subscriptable`）。以前はここで要素型を返していたので、
                    //    **必ず実行時エラーになるコードに嘘の型**を与えていた。
                    //    診断は `check_subscript_index` が出す。
                    InferredType::Set | InferredType::SetOf(_) => InferredType::Unresolved,
                    InferredType::DictOf(_, val) => *val,
                    InferredType::Tuple(types) => {
                        // リテラル整数インデックスなら対応する要素型を返す
                        if let Expr::Int(n) = index.as_ref() {
                            let i = *n as usize;
                            types.into_iter().nth(i).unwrap_or(InferredType::Unresolved)
                        } else {
                            InferredType::Unresolved
                        }
                    }
                    InferredType::Str => {
                        // 文字列の添字は文字列を返す
                        if matches!(idx_ty, InferredType::Int) { InferredType::Str } else { InferredType::Unresolved }
                    }
                    _ => InferredType::Unresolved,
                };
                // ── AST 型解決層（#16）── 添字アクセスの**要素結果型**を焼く。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }
            // ⚠ スライスの境界は `int` でなければならない（タスク 5.2b・検体 L8）。
            //    実測: `begin` / `end` は `int`・`Index`・`None`、`step` は `int`・`None`。
            Expr::Slice { begin, end, step } => {
                for (part, what) in [(begin, "begin"), (end, "end"), (step, "step")] {
                    if let Some(e) = part {
                        let ty = self.infer(e);
                        self.check_int_position(&ty, &format!("`{what}` of slice"));
                    }
                }
                InferredType::NamedInstance("slice".to_string())
            }

            // --- 型ガード式 ---
            // ⚠ 対象式に型義務は無い（`is` は検査そのもの）。型名の存在検査は
            //    妥当性検査の担当（タスク 3.4・検体 X6）。
            Expr::IsType { expr, node_id, .. } => {
                self.walk(expr);
                // `is` は Bool を返す（検査自体なので指示は不要・narrowing は直後 if 分岐で反映）。
                self.annotations.set_resolved(*node_id, InferredType::Bool);
                InferredType::Bool
            }

            // --- mustbe 動的型アサーション ---
            Expr::MustBe { expr, guard_type, span, node_id } => {
                self.infer_mustbe(expr, guard_type, span, *node_id)
            }
            Expr::Block { stmts, return_type } => {
                // ⚠ `->T` を **`block_return` の照合先**として積む（タスク 5.2）。
                let ann = Self::ann_or_none(return_type);
                self.with_block_expr(ann, |c| {
                    c.push_scope();
                    c.check_stmts(stmts);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::IfExpr {
                branches,
                else_body,
                return_type,
            } => {
                let ann = Self::ann_or_none(return_type);
                self.with_block_expr(ann, |c| {
                    for (cond, body) in branches {
                        // ⚠ 条件は `bool` でなければならない（D-12・検体 K1）。
                        c.walk_obligation_pending(cond, "5.5 条件は bool");
                        c.push_scope();
                        c.check_stmts(body);
                        c.pop_scope();
                    }
                    if let Some(body) = else_body {
                        c.push_scope();
                        c.check_stmts(body);
                        c.pop_scope();
                    }
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::ForExpr {
                target,
                iter,
                body,
                return_type,
            } => {
                let iter_ty = self.infer(iter);
                // ループ変数を要素型で宣言する（`Stmt::For` と同じ扱い）。
                // ここが抜けていたため、for **式**の本体では変数が未宣言＝`Unresolved` になり、
                // 本体の演算に型特化が効かなかった。
                let elem_ty = Self::for_element_type(&iter_ty);
                // ⚠⚠ `for` **式**も `for` 文と同じ束縛を作るので、B4 の規則を**同じく適用**する。
                //    以前は `mutable: true` 決め打ちで、内包表記の脱糖先もここを通るため
                //    文側だけ直しても穴が残っていた（実測）。
                let target_mut = self.path_is_mutable(iter).unwrap_or(false);
                // ⚠ 積むのは `->list[T]` **そのもの**。要素を取り出すのは `loop_yield`
                //    側で、内側の注釈が `list[T]` でなければ照合しない（タスク 5.2）。
                let ann = Self::ann_or_none(return_type);
                self.with_loop_expr_yielding(ann, |c| {
                    c.push_scope();
                    // 規則 1: 外側に同名があれば再束縛（`Stmt::For` と同じ形）。
                    if target != "_" && c.lookup(target).is_some() {
                        c.report_error(StaticTypeError {
                            kind: TypeErrorKind::VariableRedeclaration { name: target.clone() },
                            span: None,
                        });
                    }
                    // 規則 3: 反復対象の属性を継ぐ（一時値は `let`）。
                    c.declare(target.clone(), elem_ty, target_mut);
                    c.check_stmts(body);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::WhileExpr {
                cond,
                body,
                return_type,
            } => {
                // ⚠ 条件は `bool` でなければならない（D-12・検体 K2）。
                self.walk_obligation_pending(cond, "5.5 条件は bool");
                let ann = Self::ann_or_none(return_type);
                self.with_loop_expr_yielding(ann, |c| {
                    c.push_scope();
                    c.check_stmts(body);
                    c.pop_scope();
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::MatchExpr {
                subject,
                arms,
                return_type,
            } => {
                // ⚠ 腕の処理は `match` **文**と共有する（タスク 2.9）。以前はここに
                //    独自の走査があり `is Type` の絞り込みを持っていなかったので、
                //    **文では効く絞り込みが式では効かない**という意味論のずれがあった。
                let ann = Self::ann_or_none(return_type);
                self.with_block_expr(ann, |c| {
                    c.check_match_arms(subject, arms);
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::Cast { type_name, node_id, .. } => {
                // 挙動不変: object は従来通り infer しない（この arm は type_name のみ使う）。
                let resolved =
                    InferredType::from_ann(type_name).unwrap_or(InferredType::Unresolved);
                // ── AST 型解決層（#16）── cast は動的ディスパッチ（__cast__/変換）を伴うので
                // 解決型＝ターゲット型、検査指示＝CheckBefore(ターゲット型)。
                let tid = self.annotations.intern(resolved.clone());
                self.annotations.set_resolved(*node_id, resolved.clone());
                self.annotations
                    .set_directive(*node_id, super::annotations::Directive::CheckBefore(tid));
                resolved
            }
            Expr::DebugVar(_) => InferredType::Unresolved,
        }
    }

    /// `->Type` 注釈があれば解決した型を、なければ `Unresolved` を返す。
    /// `block`/`if`/`for`/`while`/`match` 式の結果型計算で共通に使う。
    /// コレクションリテラルの要素型を**合成**する（タスク 2.7・原因②）。
    ///
    /// - 全要素が同型 → その型
    /// - 混在 → `Union`（重複は畳む）
    /// - 1 つでも `Unresolved` → `Unresolved`（要素型が決められない）
    ///
    /// ⚠⚠ **「捨てる」ことは中立ではない。** 以前は混在を素の `List` に落としていたが、
    /// 素の `List` は `ListOf(任意)` と適合するので「**何でも通す**」になっていた。
    /// 合成すれば `[1, "s"]` は `list[Union[int, str]]` になり、`list[int]` とは適合しない。
    ///
    /// ⚠ `Unresolved` を返す場合は `ListOf(Unresolved)` になる。素の `List` と違い
    /// **構造は保つ**ので、タスク 4.1 で素の容器の双方向特例を撤去しても壊れない。
    pub(super) fn join_elem_types(types: Vec<InferredType>) -> InferredType {
        if types.iter().any(|t| matches!(t, InferredType::Unresolved)) {
            return InferredType::Unresolved;
        }
        let mut uniq: Vec<InferredType> = Vec::new();
        for t in types {
            if !uniq.contains(&t) {
                uniq.push(t);
            }
        }
        match uniq.len() {
            0 => InferredType::Unresolved,
            1 => uniq.remove(0),
            _ => InferredType::Union(uniq),
        }
    }

    /// 名前が関数を指すとき、その**関数値としての型**（タスク 2.2）。
    ///
    /// ⚠ `Expr::Ident` は変数スコープだけを引いていたので、`fn` を名前で参照した値の型が
    /// `Unresolved` になっていた。情報は `fn_sigs`（引数・戻り値）に揃っている。
    ///
    /// ⚠ **オーバーロードは `None` に倒す。** 関数値としては型が 1 つに決まらず、
    /// ここで 1 本を選ぶと嘘になる（呼び出し点は `check_call_args` が個数と型で解決する）。
    /// ⚠ **テンプレート関数も `None`。** 型引数が決まらないとシグネチャが定まらない
    /// （`f[int]` の形は `Expr::TemplateInstantiate` が扱う）。
    pub(super) fn fn_value_type(&self, name: &str) -> Option<InferredType> {
        if self
            .registry
            .template_params(name)
            .is_some_and(|p| !p.is_empty())
        {
            return None;
        }
        let sigs = self.registry.fn_sigs(name)?;
        if sigs.len() != 1 {
            return None; // オーバーロード
        }
        let sig = &sigs[0];
        // ⚠⚠ **可変長パラメータを持つ関数は型を作らない。** `FnTypeParam` は
        //    「個数が固定の引数列」しか表せないので、`f(... = 1, 2, 3)` を正しく検査できず
        //    `takes 0 argument(s) but 1 were given` という**嘘のエラー**になる（実測）。
        //    `resolve.rs` の `has_open_arity` と同じ理由・同じ判断。
        if sig.variadic_type.is_some() {
            return None;
        }
        let params = sig
            .params
            .iter()
            .zip(sig.param_mutable.iter())
            .enumerate()
            .map(|(i, ((pname, pty), pmut))| super::types::FnTypeParam {
                name: pname.clone(),
                mutable: *pmut,
                // 注釈が無い仮引数は `Any`（`from_ann` 経路と同じ既定）。
                ty: pty.clone().unwrap_or(InferredType::Any),
                has_default: i >= sig.required_count,
            })
            .collect();
        Some(InferredType::Function {
            params: Some(params),
            return_type: Box::new(sig.return_type.clone().unwrap_or(InferredType::Any)),
        })
    }

    /// 添字 `obj[i]` の**添字の型**を検査する（タスク 5.2b・検体 `L6` / `L7`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 容器 | 添字に許される型 | 外れたとき |
    /// |---|---|---|
    /// | `list` / `fixed_list` / `list_like` / `str` / `tuple` | `int` ・ `Index` ・ スライス | `TypeError` |
    /// | `dict[K, V]` | **`K` と同じ型** | `KeyError`（型エラーではない） |
    /// | `set` | **無し**（添字アクセス不可） | `TypeError` |
    ///
    /// ⚠⚠ **`bool` と `float` は添字にできない**（`xs[True]` も `xs[1.5]` も `TypeError`）。
    /// 真偽値が整数として通る言語の癖で書くと落ちるので、静的に弾く価値がある。
    ///
    /// ⚠⚠ **`dict` のキーに数値昇格は効かない。** `dict[float, int]` に `int` のキーで
    /// 引くと `KeyError` になる（実測）。等値比較は `uint → int → float` で昇格するのに
    /// **辞書引きはハッシュなので昇格しない**という食い違いがあるため、`==` と同じ規則で
    /// 判定してはいけない。
    fn check_subscript_index(&mut self, obj_ty: &InferredType, idx_ty: &InferredType) {
        use InferredType as T;
        // 添字が不透明なら判定材料が無い（取りこぼす方へ倒す）。
        // ⚠ `NamedInstance` はここで落ちる。`Index` クラスとスライス（`NamedInstance("slice")`）
        //   も通したいので、クラスは一律で素通しにするのが都合もよい。
        if Self::opaque_for_subscript(idx_ty) {
            return;
        }
        match obj_ty {
            T::List | T::ListOf(_) | T::FixedList | T::FixedListOf(_) | T::ListLike
            | T::ListLikeOf(_) | T::Str | T::Tuple(_) => {
                self.check_int_position(idx_ty, &format!("index of `{obj_ty}`"));
            }
            T::DictOf(key, _) => {
                // ⚠ キー型が不透明な辞書（`dict[Any, V]`）は判定しない。
                if Self::opaque_for_subscript(key) {
                    return;
                }
                // ⚠ 記憶域の型なので `Site::Other`（`__cast__` は挿入されない）。
                if self.types_compatible(idx_ty, key, super::type_utils::Site::Other) {
                    return;
                }
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::SubscriptTypeMismatch {
                        context: format!("key of `{obj_ty}`"),
                        expected: (**key).clone(),
                        got: idx_ty.clone(),
                    },
                    span: None,
                });
            }
            T::Set | T::SetOf(_) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NotSubscriptable { container: obj_ty.clone() },
                    span: None,
                });
            }
            // 素の `dict`（キー型不明）・クラス（`__getitem__` を定義できる）・
            // `Any` / `Unresolved` などは判定しない。
            _ => {}
        }
    }

    /// 「ここは `int` でなければならない」位置（添字・スライス境界）を検査する。
    fn check_int_position(&mut self, ty: &InferredType, context: &str) {
        if Self::opaque_for_subscript(ty) || matches!(ty, InferredType::Int) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::SubscriptTypeMismatch {
                context: context.to_string(),
                expected: InferredType::Int,
                got: ty.clone(),
            },
            span: None,
        });
    }

    /// スライス値の型か（`xs[0:2]` の添字 / `slice(...)` の戻り値）。
    fn is_slice_type(ty: &InferredType) -> bool {
        matches!(ty, InferredType::NamedInstance(n) if n == "slice")
    }

    /// 添字まわりの検査で「判定材料が無い」とみなす型。
    ///
    /// ⚠ `NamedInstance` を含めるのが要点。組み込みの `Index` クラス・スライス値
    /// （`NamedInstance("slice")`）・利用者が `__getitem__` を定義したクラスが
    /// ここを通る。`uint` 注釈も現状 `Unresolved` になるのでここで落ちる。
    fn opaque_for_subscript(ty: &InferredType) -> bool {
        use InferredType as T;
        matches!(
            ty,
            T::Unresolved
                | T::Any
                | T::Never
                | T::NamedInstance(_)
                | T::GenericInstance { .. }
                | T::SelfType
                | T::Protocol(_)
                | T::Union(_)
                | T::Intersection(_)
                | T::Namespace(_)
                | T::PyNamespace(_)
        )
    }

    /// `->T` 注釈を**照合に使える型**として返す。注釈なし・解決できない注釈は `None`
    /// （＝照合しない）。`Unresolved` を返してしまうと「何でも通る期待型」になる。
    fn ann_or_none(return_type: &Option<String>) -> Option<InferredType> {
        match return_type.as_deref().and_then(InferredType::from_ann) {
            Some(t) if !matches!(t, InferredType::Unresolved | InferredType::Any) => Some(t),
            _ => None,
        }
    }

    fn ann_or_unresolved(return_type: &Option<String>) -> InferredType {
        return_type
            .as_deref()
            .and_then(InferredType::from_ann)
            .unwrap_or(InferredType::Unresolved)
    }

    /// 属性アクセス `obj.attr` の型を推論する。`Any`/`Union`/`Result` への
    /// アクセスは診断し、名前空間メンバーはその型を直接返す。
    fn infer_attr(
        &mut self,
        object: &Expr,
        attr: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
        let obj_ty = self.infer(object);
        // ⚠ `GenericInstance{Box,[int]}` も**クラスとして扱う**（A-2）。あわせて
        //    型変数 → 具体型の置換表を取り出しておき、メンバーの型を置換してから返す。
        let class_subst = self.class_and_subst(&obj_ty).or_else(|| {
            // ⚠ **クラス名経由のアクセス**（`Color.Red` / `Counter.total` / `C.K`）も
            //    クラスのメンバー表を引く（タスク 2.3）。`Color` の型は
            //    `TypeValOf(NamedInstance("Color"))` なので `class_and_subst` では
            //    `None` になり、メンバーが全部 `Unresolved` になっていた。
            //    ⚠ enum のバリアント・`const` クラス変数・`static mut` が同じ経路に乗る。
            match &obj_ty {
                InferredType::TypeValOf(inner) => match inner.as_ref() {
                    InferredType::NamedInstance(c) => {
                        Some((c.clone(), std::collections::HashMap::new()))
                    }
                    _ => None,
                },
                _ => None,
            }
        });
        let class_name_opt = class_subst.as_ref().map(|(c, _)| c.clone());
        match &obj_ty {
            InferredType::Any => self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnAny {
                    op: "attribute access".to_string(),
                },
                span: Some(span.clone()),
            }),
            InferredType::Union(_) | InferredType::Result(_, _) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::OperationOnUnion {
                        union_type: obj_ty.to_string(),
                        op: "attribute access".to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
            // Intersection 型のメンバーアクセスはダウンキャストなしで許可する
            InferredType::Intersection(_) => {}
            _ => {}
        }
        if let Some(class_name) = &class_name_opt {
            self.check_member_access_static(class_name, attr, Some(span.clone()));
        }
        // Namespace/PyNamespace はメンバ型、それ以外は解決不能。
        let fallback = if let InferredType::Namespace(ref members) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Unresolved)
        } else if let InferredType::PyNamespace(ref members) = obj_ty {
            members.get(attr).cloned().unwrap_or(InferredType::Any)
        } else {
            InferredType::Unresolved
        };
        // ── AST 型解決層（#16）── 属性アクセスの型。`NamedInstance` のフィールドは registry から実型を引く。
        // **この実型をそのまま戻り値にする**（2026-08-10）。以前は注釈にだけ焼いて戻り値は `Unresolved` に
        // していたが、それだと `p.x * p.x` が `Unresolved` 同士の `BinOp` になり `binop_kind` が付かず、
        // VM の型特化 op もネイティブの型付き生成も効かなかった（#16 c-2 の結論「律速は型検査の解像度」）。
        // フィールドでない属性（メソッド名など）は registry に無いので従来どおり `fallback`。
        // ⚠⚠ **型引数で置換する**（A-2）。レジストリのフィールド型は置換前（`T`）なので、
        //    `Box[int]` から引いたまま返すと `T` が使用箇所へ漏れ、0-2/0-3 の照合が
        //    `declared 'T'` という偽陽性を出す（実測）。
        let resolved = match &class_subst {
            Some((class, map)) => self
                .registry
                .class_field_details(class)
                .and_then(|m| m.get(attr))
                .map(|(_, ty)| Self::subst_type_params(ty, map))
                .unwrap_or(fallback),
            None => fallback,
        };
        self.annotations.set_resolved(node_id, resolved.clone());
        resolved
    }

    /// 単項演算子の結果型を推論する。`Any`/`Union` オペランドは診断する。
    fn infer_unaryop(&mut self, op: &UnaryOp, operand: &Expr) -> InferredType {
        let ty = self.infer(operand);
        let op_str = match op {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "not",
            UnaryOp::BitNot => "~",
        };
        match &ty {
            InferredType::Any => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::OperationOnAny {
                        op: op_str.to_string(),
                    },
                    span: None,
                });
                return InferredType::Unresolved;
            }
            InferredType::Union(_) => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::OperationOnUnion {
                        union_type: ty.to_string(),
                        op: op_str.to_string(),
                    },
                    span: None,
                });
                return InferredType::Unresolved;
            }
            _ => {}
        }
        match op {
            UnaryOp::Not => InferredType::Bool,
            UnaryOp::Neg => match ty {
                InferredType::Int => InferredType::Int,
                InferredType::Float => InferredType::Float,
                InferredType::Complex => InferredType::Complex,
                _ => InferredType::Unresolved,
            },
            UnaryOp::BitNot => InferredType::Int,
        }
    }

    /// `expr mustbe Type` の型を推論する。解決した型を返し、コレクション要素型や
    /// 関数シグネチャは実行時に検査されないため警告する。
    fn infer_mustbe(
        &mut self,
        expr: &Expr,
        guard_type: &str,
        span: &Span,
        node_id: u32,
    ) -> InferredType {
        // ⚠ 対象式に型義務は無い（型ガードは検査そのもの）。
        self.walk(expr);
        let resolved = InferredType::from_ann(guard_type).unwrap_or(InferredType::Unresolved);
        // ── AST 型解決層（#16・段階(a)）──
        // `mustbe` は実行時に対象型で動的検査する（不一致で raise）。よって:
        //   解決型テーブル = 確定後の型（guard_type）／ 検査指示 = CheckBefore(その型)。
        let tid = self.annotations.intern(resolved.clone());
        self.annotations.set_resolved(node_id, resolved.clone());
        self.annotations
            .set_directive(node_id, super::annotations::Directive::CheckBefore(tid));
        // コレクション型パラメータ・関数シグネチャは実行時に未チェック → 警告
        let warn_kind = match &resolved {
            InferredType::ListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list".to_string(),
            }),
            InferredType::FixedListOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "fixed_list".to_string(),
            }),
            InferredType::ListLikeOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "list_like".to_string(),
            }),
            InferredType::SetOf(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "set".to_string(),
            }),
            InferredType::DictOf(_, _) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "dict".to_string(),
            }),
            InferredType::Tuple(_) => Some(TypeWarningKind::MustBeElemTypeUnchecked {
                guard_type: guard_type.to_string(),
                outer_type: "tuple".to_string(),
            }),
            InferredType::Function { params, return_type } => {
                let has_params = params.as_ref().is_some_and(|p| !p.is_empty());
                let has_ret = **return_type != InferredType::Any;
                if has_params || has_ret {
                    Some(TypeWarningKind::MustBeFunctionSignatureUnchecked {
                        guard_type: guard_type.to_string(),
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(kind) = warn_kind {
            self.report_warning(StaticTypeWarning { kind, span: Some(span.clone()) });
        }
        resolved
    }
}

/// 式の種類名（#16 段階 D の診断用）。`Unresolved` を生んだ式の分布を取るために使う。
fn expr_kind_name(e: &Expr) -> &'static str {
    match e {
        Expr::Ident { .. } => "Ident",
        Expr::LocalVar(_) => "LocalVar",
        Expr::Call { .. } => "Call",
        Expr::Attr { .. } => "Attr",
        Expr::TraitAccess { .. } => "TraitAccess",
        Expr::Subscript { .. } => "Subscript",
        Expr::BinOp { .. } => "BinOp",
        Expr::UnaryOp { .. } => "UnaryOp",
        Expr::Cast { .. } => "Cast",
        Expr::MustBe { .. } => "MustBe",
        Expr::TemplateInstantiate { .. } => "TemplateInstantiate",
        Expr::Block { .. } => "BlockExpr",
        Expr::IfExpr { .. } => "IfExpr",
        Expr::MatchExpr { .. } => "MatchExpr",
        Expr::ForExpr { .. } => "ForExpr",
        Expr::WhileExpr { .. } => "WhileExpr",
        Expr::Slice { .. } => "Slice",
        _ => "other",
    }
}

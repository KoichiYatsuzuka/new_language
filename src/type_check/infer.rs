use crate::ast::{Expr, UnaryOp};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use super::types::InferredType;
use super::TypeChecker;

/// **どのクラスのインスタンスにも生えているメソッド**（タスク 7.5）。
///
/// ⚠⚠ `interpreter/classes/method_call.rs` がクラスのメソッド表を引く**前に**
/// 特別扱いしているもの。実測で該当は `copy` だけ
/// （`keys` / `values` / `clear` / `items` / `get` / `add` / `append` / `pop` は
/// どれも `AttributeError: 'C' has no method ...`）。
/// ⚠ 実装側に共通メソッドを足したら**ここにも足すこと**。足し忘れると
/// 正しいコードが「存在しないメンバー」で落ちる。
const UNIVERSAL_INSTANCE_METHODS: &[&str] = &["copy"];

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

    /// 式の型を推論して [`InferredType`] を返す。副作用として型エラーを収集する場合がある。
    ///
    /// ⚠⚠ **`#[must_use]`**（タスク 3.1）。結果を捨てたいときは [`Self::walk`] を
    /// 使うこと。直接 `self.infer(e);` と書くと警告になる ＝ 「義務を忘れた」のか
    /// 「歩くだけ」なのかを宣言させる仕掛け。
    ///
    /// ⚠ タスク 3.1 では「義務があるが未実装」を表す 2 つ目のラッパも置いていたが、
    /// **フェーズ 5 で印を付けた地点をすべて実装し終えた**ので撤去した
    /// （残っていると「義務が無い」と区別できない死んだ足場になる）。
    /// 外側の網は `scripts/type_obligations.ps1`（114 件の検体）が引き続き担う。
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
                    let types = self.seq_entry_elem_types(elems);
                    InferredType::ListOf(Box::new(Self::join_elem_types(types)))
                }
            }
            Expr::Set(elems) => {
                if elems.is_empty() {
                    InferredType::SetOf(Box::new(InferredType::Never))
                } else {
                    let types = self.seq_entry_elem_types(elems);
                    InferredType::SetOf(Box::new(Self::join_elem_types(types)))
                }
            }
            Expr::Tuple(exprs) => {
                // ⚠ タプルは**位置ごとの型**を持つが、`*other` を含むと長さが実行時に
                //   決まるので位置が対応づけられない。⇒ 展開があれば `TupleAny` に倒す
                //   （要素型を捨てるのではなく「位置が判らない」ことを表す）。
                if exprs.iter().any(|e| e.is_spread()) {
                    InferredType::TupleAny
                } else {
                    let types: Vec<InferredType> =
                        exprs.iter().map(|e| self.infer(e.expr())).collect();
                    InferredType::Tuple(types)
                }
            }

            // --- 属性アクセス ---
            Expr::Attr { object, attr, span, node_id, .. } => {
                self.infer_attr(object, attr, span, *node_id)
            }
            // ⚠⚠ **trait 修飾アクセス `o::T.f` は型を返していなかった**（タスク 5.2d・検体 `F3`）。
            //    `Unresolved` は「何でも通る」ので、読みも代入も**丸ごと無検査**だった
            //    （`c::T.n = "s"` は A-3 で入れた実行時検査だけが止めていた）。
            //    宣言は `trait_field_details` に揃っているので、そこから引く。
            // ⚠ メソッド（`o::T.m()`）はここでは解決しない。呼び出しの検査は
            //    `infer_call` の別経路で、そちらを変えると影響範囲が広い。
            Expr::TraitAccess { object, trait_name, attr } => {
                self.walk(object);
                match self
                    .registry
                    .trait_field_details(trait_name.as_str())
                    .and_then(|f| f.get(attr.as_str()))
                {
                    Some((_, ty)) => ty.clone(),
                    None => InferredType::Unresolved,
                }
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
            Expr::Dict(entries) => {
                if entries.is_empty() {
                    InferredType::DictOf(
                        Box::new(InferredType::Never),
                        Box::new(InferredType::Never),
                    )
                } else {
                    // ⚠ キー・値それぞれを合成する（タスク 2.7）。以前はどちらかが
                    //    揃わないだけで素の `Dict` に落ち、`dict[任意, 任意]` と適合していた。
                    // ★ `**other`（`DictEntry::Spread`）は**展開元の要素型を持ち込む**。
                    //    合成に混ぜることで `{**b, 3.0: 2}` のようなキー型の食い違いが
                    //    そのまま代入時の検査で捕まる。
                    let mut key_types: Vec<InferredType> = Vec::new();
                    let mut val_types: Vec<InferredType> = Vec::new();
                    for e in entries {
                        match e {
                            crate::ast::DictEntry::Pair(k, v) => {
                                key_types.push(self.infer(k));
                                val_types.push(self.infer(v));
                            }
                            crate::ast::DictEntry::Spread(src) => {
                                let (kt, vt) = self.dict_spread_elem_types(src);
                                key_types.push(kt);
                                val_types.push(vt);
                            }
                        }
                    }
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
                let ann = self.ann_or_none(return_type);
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
                let ann = self.ann_or_none(return_type);
                self.with_block_expr(ann, |c| {
                    for (cond, body) in branches {
                        // ⚠ 条件は `bool` でなければならない（D-12・検体 K1）。
                        c.check_condition_is_bool(cond, "if");
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
                targets,
                iter,
                body,
                return_type,
            } => {
                let iter_ty = self.infer(iter);
                // ⚠ 反復できない型を弾く（タスク 7.1・検体 `K3`）。文と同じ検査を通す。
                self.check_iterable(&iter_ty, None);
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
                let ann = self.ann_or_none(return_type);
                // ⚠ 多ターゲット（`for k, v in d.items()`）の型の割り付けは
                //   **`Stmt::For` と同じ規則**にする（片方だけ直すとずれる）。
                //   ⚠⚠ 対応が付かないときは **`Stmt::For` と同じく `Unresolved`**。
                //     `Any` に倒すと「明示ダウンキャスト必須」になり、
                //     `for i, c in enumerate(xs)` のように**要素型が判らない反復対象**で
                //     `str(i) + c` のような普通の式が落ちる（実測）。
                //     ⇒ ここを `Any` にするより、**反復対象の型を判るようにする方が筋**。
                //     `d.items()` は `builtin_collection_method_return` で
                //     `list[tuple[K, V]]` が付くようになったので、この枝には落ちない。
                //     `enumerate` / `zip` の戻り値型は未整備（起票候補）。
                let target_tys: Vec<InferredType> = match (&elem_ty, targets.len()) {
                    (_, 1) => vec![elem_ty.clone()],
                    (InferredType::Tuple(ts), n) if ts.len() == n => ts.clone(),
                    (_, n) => vec![InferredType::Unresolved; n],
                };
                self.with_loop_expr_yielding(ann, |c| {
                    c.push_scope();
                    for (target, ty) in targets.iter().zip(target_tys) {
                        // 規則 1: 外側に同名があれば再束縛（`Stmt::For` と同じ形）。
                        if target != "_" && c.lookup(target).is_some() {
                            c.report_error(StaticTypeError {
                                kind: TypeErrorKind::VariableRedeclaration {
                                    name: target.clone(),
                                },
                                span: None,
                            });
                        }
                        // 規則 3: 反復対象の属性を継ぐ（一時値は `let`）。
                        c.declare(target.clone(), ty, target_mut);
                    }
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
                self.check_condition_is_bool(cond, "while");
                let ann = self.ann_or_none(return_type);
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
                let ann = self.ann_or_none(return_type);
                self.with_block_expr(ann, |c| {
                    c.check_match_arms(subject, arms);
                });
                Self::ann_or_unresolved(return_type)
            }
            Expr::Cast { object, type_name, span, node_id } => {
                // ⚠ タスク 7.3 で**対象式も推論する**ようにした（以前は「挙動不変」のため
                //    推論していなかった）。成功しうる組み合わせが 1 つも無いキャストを
                //    弾くのに元の型が要る。
                let src_ty = self.infer(object);
                let resolved =
                    InferredType::from_ann(type_name).unwrap_or(InferredType::Unresolved);
                self.check_cast_possible(&src_ty, type_name, &resolved, span);
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
    /// 列リテラル（`list` / `set`）の要素型を集める。`*other` は**展開元の要素型**を持ち込む。
    ///
    /// ⚠⚠ **`Unresolved` を返さない**（`dict_spread_elem_types` と同じ判断）。
    /// `Unresolved` は下流で「万能受容体」になり、要素型の食い違いを素通しさせてしまう。
    /// 判らないときは `Any` に倒す —— こちらは明示ダウンキャストを要求する側なので
    /// 検査が消えない。
    ///
    /// ⚠ 展開元が反復できない型なら**その場で静的エラー**（実行時と同じ意味）。
    fn seq_entry_elem_types(&mut self, entries: &[crate::ast::SeqEntry]) -> Vec<InferredType> {
        use InferredType as IT;
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            match e {
                crate::ast::SeqEntry::Item(x) => out.push(self.infer(x)),
                crate::ast::SeqEntry::Spread(x) => {
                    let ty = self.infer(x);
                    out.push(match &ty {
                        IT::ListOf(t) | IT::FixedListOf(t) | IT::ListLikeOf(t) | IT::SetOf(t) => {
                            (**t).clone()
                        }
                        // 要素型を持たない容器。反復はできるので `Any`。
                        IT::List | IT::FixedList | IT::ListLike | IT::Set | IT::TupleAny => IT::Any,
                        // タプルは位置ごとの型を持つ。展開すると位置が崩れるので合成する。
                        IT::Tuple(ts) => Self::join_elem_types(ts.clone()),
                        // `str` を展開すると 1 文字ずつの `str`。
                        IT::Str => IT::Str,
                        IT::Any | IT::Unresolved => IT::Any,
                        // クラスは `__iter__` を持ちうるので通す（実行時に判る）。
                        IT::NamedInstance(_) => IT::Any,
                        other => {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::SpreadArgNotIterable {
                                    got: other.to_string(),
                                    kw: false,
                                },
                                span: None,
                            });
                            IT::Any
                        }
                    });
                }
            }
        }
        out
    }

    /// 辞書リテラル内の `**src` が持ち込む（キー型, 値型）を返す。
    ///
    /// ⚠⚠ **`Unresolved` を返さない**。`Unresolved` は下流で「万能受容体」として
    /// 振る舞い、`{**b, 3.0: 2}` のようなキー型の食い違いを**素通し**させてしまう。
    /// 判らないときは `Any` に倒す —— `Any` は「明示ダウンキャストを要求する」側なので、
    /// 検査を消さずに済む（`extract_py_type_stubs` の doc と同じ判断）。
    ///
    /// ⚠ 展開元が辞書でなければ**その場で静的エラー**（実行時 `TypeError` と同じ形）。
    fn dict_spread_elem_types(
        &mut self,
        src: &Expr,
    ) -> (InferredType, InferredType) {
        let ty = self.infer(src);
        match &ty {
            InferredType::DictOf(k, v) => ((**k).clone(), (**v).clone()),
            // 要素型を持たない素の `dict`。要素は判らないが**辞書ではある**ので
            // `Any` として通す（`Unresolved` にすると検査が消える）。
            InferredType::Dict => (InferredType::Any, InferredType::Any),
            // ⚠ 型が判らない（`Any` / `Unresolved`）ときは検査できないが、
            //   **エラーにもしない**（`import` 越しの値など、注釈が供給されない経路がある）。
            InferredType::Any | InferredType::Unresolved => {
                (InferredType::Any, InferredType::Any)
            }
            other => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::DictSpreadNotADict {
                        got: other.to_string(),
                    },
                    span: None,
                });
                (InferredType::Any, InferredType::Any)
            }
        }
    }

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
    /// ブロック式（`if:` / `match:` / `block:` / `for` / `while`）の `->T` 注釈を解く。
    ///
    /// ⚠⚠ **素の容器型注釈の検査（タスク 8.1・案 A）をここに置いてある。**
    /// ブロック式の `->T` は 5 箇所から作られるが、**全部この 1 本を通る**ので、
    /// ここに置けば `->list` の書き漏らしを一つ残らず捕まえられる。
    /// ⇒ 呼び出し側で個別に検査を足さないこと（足すと二重に鳴る）。
    fn ann_or_none(&mut self, return_type: &Option<String>) -> Option<InferredType> {
        if let Some(ann) = return_type.as_deref() {
            self.check_ann_not_bare(ann, "block expression result");
        }
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
            // ⚠ メンバーの存在検査（タスク 7.5・検体 `M1` / `M2`）。
            if !self.member_exists(class_name, attr) {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::NoSuchMember {
                        class_name: class_name.clone(),
                        member: attr.to_string(),
                    },
                    span: Some(span.clone()),
                });
            }
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

    /// `class_name` が `member` を持つか（タスク 7.5・検体 `M1` / `M2`）。
    ///
    /// ## ⚠⚠ Arrow のメンバーは**クラス本体の宣言だけで確定する**
    ///
    /// 実行時の文言がそう言っている:
    ///
    /// ```text
    /// AttributeError: 'Box' has no field 'newattr'; all fields must be declared in the class body
    /// ```
    ///
    /// `__init__` の中でも、他のメソッドでも、外からでも**宣言の無いフィールドは作れない**。
    /// ⇒ Arrow のクラスに関して存在検査は**完全に静的に決まる**。
    ///
    /// ## 判定の順序
    ///
    /// 1. **protocol** … 要求メンバー（`registry.protocol`）を見る
    /// 2. **フィールド** … 自クラス ＋ クラス継承 ＋ 基底 trait（`declared_field_type`）
    /// 3. **メソッド** … 継承チェーン込み ＋ 基底 trait のメソッド
    /// 4. **メンバー情報を 1 つも持たないクラス** … **開いている**とみなして素通し（下記）
    ///
    /// ## ⚠⚠ 「情報が無い」と「メンバーが無い」を取り違えないこと
    ///
    /// レジストリにフィールド表もメソッド表も基底も無いクラスは、
    /// **「メンバーが無い」のではなく「こちらが知らない」**。素通しする。実際に該当したもの:
    ///
    /// | 例 | 理由 |
    /// |---|---|
    /// | `slice` | 組み込みクラス。`begin`/`end`/`step` は `eval/attrs.rs` の特別扱い |
    /// | 関数の中で宣言した `enum` | 収集パスが拾っていない |
    /// | テンプレート型変数 `T` | `NamedInstance("T")` に化けているだけ |
    ///
    /// ⚠ これがタスク 7.8 で設計した `member_set_is_closed`（メンバー集合が閉じているか）の
    /// 実体。**スタブが入ったときに「スタブに載っているものだけ許す」へ寄せられる形**に
    /// してある。真偽値ではなく**クラス単位の性質**として持つこと。
    pub(super) fn member_exists(&self, class_name: &str, member: &str) -> bool {
        // 0. ⚠⚠ **全インスタンス共通のメソッド**。`method_call.rs` がクラスのメソッド表を
        //    引く**前に**特別扱いしているので、どのクラスにも生えている。
        //    実測で確かめた（`copy` だけが該当。`keys` / `values` / `clear` / `items` /
        //    `get` / `add` / `append` / `pop` はどれも `AttributeError`）。
        // ⚠ 実装側に共通メソッドを足したら**ここにも足すこと**。
        if UNIVERSAL_INSTANCE_METHODS.contains(&member) {
            return true;
        }
        // 1. protocol は要求メンバーが別の表にある。
        if let Some(p) = self.registry.protocol(class_name) {
            return p.fields.iter().any(|f| f.name == member)
                || p.methods.iter().any(|m| m.name == member);
        }
        // 2/3. フィールドとメソッド。
        if self.declared_field_type_pub(class_name, member).is_some() {
            return true;
        }
        if self.collect_class_method_sigs(class_name).contains_key(member) {
            return true;
        }
        for base in self.registry.class_bases(class_name).unwrap_or(&[]) {
            if self
                .registry
                .trait_methods(base)
                .is_some_and(|m| m.contains_key(member))
            {
                return true;
            }
            if self
                .registry
                .trait_field_details(base)
                .is_some_and(|m| m.contains_key(member))
            {
                return true;
            }
        }
        // 4. メンバー集合が閉じていない（＝こちらが何も知らない）クラスは素通し。
        !self.member_set_is_closed(class_name)
    }

    /// クラスの**メンバー集合が閉じている**か（タスク 7.5 / 7.8）。
    ///
    /// ⚠⚠ **真偽値に潰さずクラス単位の性質として持つこと。** 外部言語のスタブが入ったとき、
    /// 言語によって「スタブ＝完全な宣言（閉じる）」「スタブ＝部分的な宣言（開いたまま）」が
    /// 分かれる。ここを「外部由来なら飛ばす」にすると**その区別が表現できなくなる**。
    ///
    /// 現状の規則: **レジストリに何らかのメンバー情報があれば閉じている**。
    /// Arrow のクラス宣言（`.ar` / `.ars` スタブ由来を含む）は必ず表を持つので閉じる。
    fn member_set_is_closed(&self, class_name: &str) -> bool {
        self.registry.class_field_details(class_name).is_some()
            || self.registry.class_methods(class_name).is_some()
            || self
                .registry
                .class_bases(class_name)
                .is_some_and(|b| !b.is_empty())
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
        // ⚠⚠ **可否と結果型を同じ表で決める**（4.4 の `check_arith` と同じ形・タスク 7.1）。
        //    以前は「不可の組み合わせ」でも黙って `Unresolved`（`BitNot` に至っては
        //    無条件 `Int`）を返しており、**型が判っているのに誤りを報告しない**
        //    ＝根本原因①（`Unresolved` は万能受容体）がここに残っていた。
        //
        // ## 実測した実行時の規則
        //
        // | 演算子 | 許される型 | 外れたとき |
        // |---|---|---|
        // | `-`  | `int` ・ `float` ・ `complex` | `bad operand type for unary '-'` |
        // | `~`  | `int` のみ | `bad operand type for unary '~'` |
        // | `not`| 何でも（`eval_truthy` を通る） | — |
        //
        // ⚠⚠ **`bool` は `-` も `~` も不可**（`-True` / `~True` は実行時 TypeError）。
        //    整数として通る言語の癖で書くと落ちるので静的に弾く価値がある。
        // ⚠ クラスは `__neg__` を定義できる（実測）ので素通しする。
        if matches!(op, UnaryOp::Not) {
            return InferredType::Bool;
        }
        if Self::opaque_for_unary(&ty) {
            return InferredType::Unresolved;
        }
        let result = match op {
            UnaryOp::Neg => match ty {
                InferredType::Int => Some(InferredType::Int),
                InferredType::Float => Some(InferredType::Float),
                InferredType::Complex => Some(InferredType::Complex),
                _ => None,
            },
            UnaryOp::BitNot => match ty {
                InferredType::Int => Some(InferredType::Int),
                _ => None,
            },
            UnaryOp::Not => unreachable!("`not` は上で返している"),
        };
        match result {
            Some(t) => t,
            None => {
                self.report_error(StaticTypeError {
                    kind: TypeErrorKind::IncompatibleUnaryOp {
                        op: op_str.to_string(),
                        operand: ty,
                    },
                    span: None,
                });
                InferredType::Unresolved
            }
        }
    }

    /// `mustbe` が**成功しうるか**を検査する（タスク 7.3・検体 `T2`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 元 | 先 | 結果 |
    /// |---|---|---|
    /// | `Any` / `Union` | 具体型 | ✅（これが `mustbe` の本来の用途） |
    /// | trait | 実装クラス | ✅（ダウンキャスト） |
    /// | `int` | `int` | ✅ |
    /// | `int` | `str` | ⛔ |
    /// | **`int`** | **`float`** | ⛔ **数値昇格は効かない** |
    ///
    /// ⚠⚠ **`mustbe` は実行時の型そのものを見る**ので、`==` の昇格ラティス
    /// （`uint → int → float`）は**効かない**。`1 mustbe float` は必ず失敗する。
    /// タスク 5.4（`case` パターン）と規則が違うので混同しないこと。
    ///
    /// ⚠ 判定は**実行時の種別**（`value_matches_type_ann` が見る外側の種別）で行う。
    /// `list[int] mustbe list[str]` は外側が同じなので**素通し**（要素型は既存の警告が扱う）。
    fn check_mustbe_possible(
        &mut self,
        src_ty: &InferredType,
        target_ty: &InferredType,
        span: &Span,
    ) {
        let (Some(a), Some(b)) = (
            Self::runtime_kind_name(src_ty),
            Self::runtime_kind_name(target_ty),
        ) else {
            return;
        };
        if a == b {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::MustBeCanNeverSucceed {
                from: src_ty.clone(),
                to: target_ty.clone(),
            },
            span: Some(span.clone()),
        });
    }

    /// 実行時に区別される**種別名**。判定できない型は `None`（＝素通し）。
    ///
    /// ⚠ クラス・trait・protocol・`Any` / `Union` などは `None`。ダウンキャストが
    /// 成立しうるので種別で弾いてはいけない。
    fn runtime_kind_name(ty: &InferredType) -> Option<&'static str> {
        use InferredType as T;
        Some(match ty {
            T::Int => "int",
            T::Float => "float",
            T::Complex => "complex",
            T::Str => "str",
            T::Bool => "bool",
            T::None => "None",
            T::Undefined => "Undefined",
            T::List | T::ListOf(_) => "list",
            T::FixedList | T::FixedListOf(_) => "fixed_list",
            T::Dict | T::DictOf(_, _) => "dict",
            T::Set | T::SetOf(_) => "set",
            T::Tuple(_) => "tuple",
            _ => return None,
        })
    }

    /// `=>` キャストが**成功しうるか**を検査する（タスク 7.3・検体 `T1`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 元 | 先 | 結果 |
    /// |---|---|---|
    /// | クラスのインスタンス | `__cast__[T]` を持つ `T` | ✅ |
    /// | クラスのインスタンス | `__cast__[T]` を**持たない** `T` | ⛔ `'C' is not castable to 'int'` |
    /// | 何でも | **`new_type`** | ✅ |
    /// | `list` | **`fixed_list[T]`** | ✅（平坦化変換） |
    /// | それ以外（`1 => float` ・ `"5" => int` ・ `xs => list[int]`） | — | ⛔ `requires an instance or new_type target` |
    ///
    /// ⚠⚠ **プリミティブ同士の `=>` は書けない**（`1 => float` は実行時エラー）。
    /// 変換は `float(1)` のような組み込み関数を使う（決定 D-5 / タスク 2.4）。
    ///
    /// ⚠ 判定材料が無い型（`Unresolved` / `Any` / `Union` …）は素通し。
    /// ⚠ `__cast__` は**継承チェーンを辿って**探す（基底クラス・trait 由来も有効）。
    fn check_cast_possible(
        &mut self,
        src_ty: &InferredType,
        target_name: &str,
        target_ty: &InferredType,
        span: &Span,
    ) {
        use InferredType as T;
        if Self::opaque_for_unary(src_ty) && !matches!(src_ty, T::NamedInstance(_) | T::GenericInstance { .. })
        {
            return;
        }
        // `new_type` 先は何からでも作れる。
        if self.registry.new_type_original(target_name).is_some() {
            return;
        }
        // 解決できない先（trait / protocol / 未知の名前）は判定しない。
        if matches!(target_ty, T::Unresolved | T::Any | T::Protocol(_)) {
            return;
        }
        if self.registry.is_known_trait(target_name) || self.registry.is_protocol(target_name) {
            return;
        }
        // ⚠⚠ **`new_type` からの取り出しは許される**（`Count => int` など）。
        //    `scan_examples` が `other_typing.ar` / `polymorphism.ar` で教えてくれた。
        //    `new_type` のラッパは `__cast__` を持たないが、実行時は基底型へ戻せる。
        if let T::NamedInstance(name) = src_ty {
            if self.registry.new_type_original(name.as_str()).is_some() {
                return;
            }
        }
        // クラスのインスタンス → `__cast__[T]` の有無で決まる。
        if let Some((cls, _)) = self.class_and_subst(src_ty) {
            // ⚠ テンプレートクラスなどで置換表が作れない場合も名前で引ければ十分。
            let key = format!("__cast__[{target_name}]");
            if self.collect_class_method_sigs(cls.as_str()).contains_key(&key) {
                return;
            }
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::CastCanNeverSucceed {
                    from: src_ty.clone(),
                    to: target_name.to_string(),
                    reason: format!("no `{key}` method defined"),
                },
                span: Some(span.clone()),
            });
            return;
        }
        // ⚠⚠ `list` ⇄ `fixed_list` は**双方向**に許される（平坦化変換とその復元）。
        //    `scan_examples` が `fixed_list.ar` / `runtime_checks_in_function.ar` で
        //    `fl => list[Vec2]` を落として教えてくれた（片方向しか書いていなかった）。
        let list_like_kind = |t: &T| {
            matches!(
                t,
                T::List | T::ListOf(_) | T::FixedList | T::FixedListOf(_)
                    | T::ListLike | T::ListLikeOf(_)
            )
        };
        if list_like_kind(src_ty) && list_like_kind(target_ty) {
            return;
        }
        // ここまで来たら「インスタンスでもなく、先が new_type でも fixed_list でもない」。
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::CastCanNeverSucceed {
                from: src_ty.clone(),
                to: target_name.to_string(),
                reason: "`=>` requires an instance source or a new_type target".to_string(),
            },
            span: Some(span.clone()),
        });
    }

    /// 単項演算子の検査で「判定材料が無い」とみなす型（タスク 7.1）。
    ///
    /// ⚠ クラス（`__neg__` を定義できる）を含めるのが要点。
    fn opaque_for_unary(ty: &InferredType) -> bool {
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
                | T::Result(_, _)
                | T::Namespace(_)
                | T::PyNamespace(_)
                | T::TypeVal
                | T::TypeValOf(_)
        )
    }

    /// `for` の反復対象が反復可能かを検査する（タスク 7.1・検体 `K3`）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 反復できる | 反復できない |
    /// |---|---|
    /// | `list` / `fixed_list` / `list_like` / `set` / `tuple` / `str` | `int` ・ `float` ・ `bool` |
    /// | `range(..)` ・ ジェネレータ | **`dict`** |
    /// | `__iter__` を持つクラス | `__iter__` を持たないクラス（`AttributeError`） |
    ///
    /// ⚠⚠ **`dict` は反復できない**（`for k in d:` は `TypeError: object is not iterable`）。
    /// Python と違うので、ここを取りこぼすと Python の癖で書いた `for k in d:` が
    /// 実行時まで判らない。
    /// ⚠ クラスは `__iter__` を定義できるので素通しする（持たない場合は実行時 `AttributeError`）。
    pub(super) fn check_iterable(&mut self, iter_ty: &InferredType, span: Option<Span>) {
        use InferredType as T;
        // 判定できるのは「確実に反復できない」と分かっている型だけ。
        let not_iterable = matches!(
            iter_ty,
            // ⚠ **`dict` は反復できる**（キーが出る・Python と同じ）。以前はここで
            //   弾いていたので `for k in d:` が `'dict[..]' is not iterable` だった。
            T::Int | T::Float | T::Complex | T::Bool | T::None | T::Undefined
        );
        if !not_iterable {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::NotIterable { ty: iter_ty.clone() },
            span,
        });
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
        // ⚠ タスク 7.3 で**対象式の型を使う**ようにした（以前は `walk` で捨てていた）。
        //    「成功しうる値が 1 つも無い」表明を弾くのに元の型が要る。
        let src_ty = self.infer(expr);
        let resolved = InferredType::from_ann(guard_type).unwrap_or(InferredType::Unresolved);
        self.check_mustbe_possible(&src_ty, &resolved, span);
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

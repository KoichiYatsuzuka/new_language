use crate::ast::{Expr, FieldKind, UnaryOp};
use crate::token::Span;

use super::errors::{StaticTypeError, StaticTypeWarning, TypeErrorKind, TypeWarningKind};
use super::types::InferredType;
use super::TypeChecker;

mod fn_types;
mod containers;
mod attr;
mod names;
mod unary;
mod conversions;

/// クラス側に置いたディスクリプタの型（[`TypeChecker::class_descriptor`]・タスク 1-12）。
pub(crate) struct ClassDescriptor {
    /// 読んだ値の型（`__get__` の戻り値）。
    pub(crate) read: InferredType,
    /// データディスクリプタ（`__set__` か `__delete__` を持つ）か。インスタンス経由の代入が `__set__` へ行く。
    pub(crate) is_data: bool,
    /// 代入する値の型（`__set__` の `value`）。`__set__` が無ければ `None`。
    pub(crate) write: Option<InferredType>,
    /// メンバーの宣言型（ディスクリプタのクラス）。クラス経由の代入（`C.x = d`）はこれを置き換える。
    pub(crate) declared: InferredType,
}

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
    ///
    /// ⚠ 推論のあいだは、式の**錨**（`Expr::anchor_span`）を位置を持たない誤りの位置として控える
    ///   （`expr_anchor`・diagnostic_position_plan.md の 2-1）。錨を持たない式は外側の錨をそのまま使う。
    #[must_use]
    pub(super) fn infer(&mut self, expr: &Expr) -> InferredType {
        let Some(anchor) = expr.anchor_span() else { return self.infer_inner(expr) };
        let outer = self.expr_anchor.replace(anchor.clone());
        let ty = self.infer_inner(expr);
        self.expr_anchor = outer;
        ty
    }

    /// [`Self::infer`] の本体（錨の張り替えを除いたもの）。
    fn infer_inner(&mut self, expr: &Expr) -> InferredType {
        match expr {
            // --- リテラル ---
            // `code:` が作るのは `Code`（設計書 §1.2）。
            // ⚠ メタ関数の外へは持ち出せない。束縛点で弾く（`check_var_decl`）。
            Expr::CodeBlock(_) => InferredType::Code,
            // ⚠ `meta_*` 型は 1-5 で入れる。それまでは未解決（嘘の型を作らない）。
            Expr::MetaInfo(_) => InferredType::Unresolved,
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
            Expr::Attr { object, attr, span, attr_span, node_id, .. } => {
                // 属性の誤りは**属性名**を指す（`.` ではない・diagnostic_position_plan.md の 3-5）。合成した属性参照（位置 0）は `.`。
                let at = if attr_span.line != 0 { attr_span } else { span };
                self.infer_attr(object, attr, at, *node_id)
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
                // 受け手・引数が不変と証明できた経路なら記録する（3-1。`let self` / `let` 仮引数で受けるときに複製しない）
                if let Expr::Attr { object, .. } | Expr::TraitAccess { object, .. } = func.as_ref() {
                    self.note_immutable_path(object);
                }
                for a in args {
                    if matches!(a, crate::ast::CallArg::Positional(_) | crate::ast::CallArg::Keyword { .. }) {
                        self.note_immutable_path(a.expr());
                    }
                }
                // 受け手・ほかの引数と重ならない位置引数の印（4-6）。全部が位置引数の呼び出しだけ
                if !args.is_empty()
                    && args.len() < 30
                    && args.iter().all(|a| matches!(a, crate::ast::CallArg::Positional(_)))
                {
                    let receiver = match func.as_ref() {
                        Expr::Attr { object, .. } => Some(object.as_ref()),
                        _ => None,
                    };
                    let mut mask = 0u32;
                    for (i, a) in args.iter().enumerate() {
                        let others: Vec<&Expr> = receiver
                            .into_iter()
                            .chain(args.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.expr()))
                            .collect();
                        let mut immutable = |e: &Expr| self.path_is_mutable(e) == Some(false);
                        if super::annotations::arg_is_disjoint(a.expr(), &others, &mut immutable) {
                            mask |= 1 << i;
                        }
                    }
                    self.annotations.set_disjoint_args(*node_id, mask);
                }
                // ── AST 型解決層（#16）── 呼び出しの**結果型**を焼く（CallInfo は infer_call_inner が充填）。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }

            // --- 識別子 ---
            // 記憶域の解決（`res`）はリゾルバが型検査の後に書くので、ここでは常に未解決。
            // 型検査は名前でスコープを引くだけで `res` を見ない。
            Expr::Ident { name, node_id, .. } => {
                let in_scope = self.lookup(name).map(|v| v.ty.clone());
                let binding = self.lookup(name).and_then(|v| v.binding);
                // どのスコープにも無い名前（フェーズ10 10-12）。以前は `Unresolved` に倒して素通しにしていた。
                // ⚠ 関数表（`fn_sigs`）で型が引けても見る（name_scope_plan.md の 1-4）。関数表はプログラム全体の
                //   表で、別の関数の中の入れ子の関数・後で定義する関数も載っている（§0 の A7・B3）。
                let visible = in_scope.is_some() || self.check_name_defined(name);
                // 使っている位置の結び付き（エディタだけ・2-2）。束縛の記録がある変数と、見えない名前だけを記録する。
                if !self.in_instance_decl() {
                    if let Some(uses) = self.uses.as_mut() {
                        if !visible {
                            uses.push(super::NameUse { node_id: *node_id, binding: None });
                        } else if let Some(b) = binding {
                            uses.push(super::NameUse { node_id: *node_id, binding: Some(b) });
                        }
                    }
                }
                let found = in_scope
                    // ⚠⚠ **変数スコープに無ければ関数を探す**（タスク 2.2）。
                    //    以前はここで `Unresolved` に倒していたため、`fn` を名前で参照した
                    //    **関数値に型が付いていなかった**。`Unresolved` は
                    //    `type_matches_exact` の万能受容体なので
                    //      let x: int = wrong                       # int 変数に関数が入る
                    //      let f: function[int]->int = takes_str    # シグネチャ違いが通る
                    //    が黙って通っていた。引数・戻り値は `fn_sigs` に揃っている。
                    .or_else(|| self.fn_value_type(name.as_str()));

                // `from h import[cpp-dll] f` した、今の Arrow から呼べない C の関数（1-2）。
                self.check_ffi_unsupported_use(name, name, None);
                let result = found.unwrap_or(InferredType::Unresolved);
                // ── AST 型解決層（#15b）── 参照サイトごとの型を焼く。
                // 変数単位ではなく**参照位置単位**なのが要点で、型ガード絞り込みは
                // 分岐スコープでの再 `declare` として実装されているため、同じ変数でも
                // 参照位置によって `lookup` の答えが変わる。
                self.annotations.set_resolved(*node_id, result.clone());
                result
            }

            // --- local::name 変数 ---
            // `local::` の名前空間に束縛されるのは可変長パラメータの `local::args` だけ（`declare_param_in`）。
            // ⚠ 見つからない名前は静的な誤り（known_bugs.md #22）。以前は `Unresolved` に倒して素通しにしていたので、
            //   `local::calls`（`static mut calls` のつもり）が型検査を通り、VM のコンパイラが枠に載せられず
            //   `VmForceError` で落ちていた。
            Expr::LocalVar(name) => {
                let key = format!("local::{}", name);
                match self.lookup(&key).map(|v| v.ty.clone()) {
                    Some(ty) => ty,
                    None => {
                        if !self.registry_incomplete && !self.already_reported_undefined(&key) {
                            self.report_error(StaticTypeError {
                                kind: TypeErrorKind::UndefinedName { name: key },
                                span: None,
                            });
                        }
                        InferredType::Unresolved
                    }
                }
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
                // `m.Box[int]` も `Box` と読む（タスク 2-15・`template_base_name` の doc）。
                let Some(name) = self.template_base_name(base) else {
                    return InferredType::Unresolved;
                };
                let name = name.to_string();
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
                let idx_ty = if self.takes_foreign_slices(&obj_ty) {
                    self.infer_foreign_index(index)
                } else {
                    self.infer(index)
                };
                // ⚠ 添字の型を検査する（タスク 5.2b・検体 L6 / L7）。
                self.check_subscript_index(&obj_ty, &idx_ty);
                // CPython の物の添字に Arrow の list などをキャストせずに使わない（`df[["a", "b"]]`・D-12・2-11）
                if self.is_python_value(&obj_ty) {
                    self.check_no_arrow_collection_to_python(&idx_ty, format!("subscript of `{obj_ty}`"));
                }
                // ⚠⚠ **スライス添字は要素ではなく「同じ種類の容器」を返す**（タスク 5.2b）。
                //    以前は添字がスライスでも要素型を返していたので
                //    `let y: int = xs[0:2]` が**黙って通り**、実行時は `[1, 2]` が入っていた。
                //    実測（`eval_subscript_slice`）: list→list ／ str→str ／ tuple→tuple ／
                //    クラスは `__getitem__` へ委譲 ／ それ以外は実行時 TypeError。
                if Self::is_slice_type(&idx_ty) {
                    let result = match &obj_ty {
                        // `py_list` の切り出しは CPython の新しい list（D-12・2-7）
                        InferredType::List
                        | InferredType::ListOf(_)
                        | InferredType::PyList(_)
                        | InferredType::Str
                        | InferredType::Bytes => obj_ty.clone(),
                        // ⚠ tuple は**要素数が変わる**ので静的には決められない。
                        // ⚠ fixed_list / list_like / set / dict はスライスできない
                        //    （実行時 TypeError）。診断は出さず判定不能に倒す。
                        _ => InferredType::Unresolved,
                    };
                    self.annotations.set_resolved(*node_id, result.clone());
                    return result;
                }
                // protocol の型の値（`xs: Sequence[int]`）は `__getitem__` の要求のシグネチャで検査する（6-7）
                if let InferredType::Protocol(p, pargs) = self.resolve_protocols(&obj_ty) {
                    let result = self.protocol_subscript(&p, &pargs, &obj_ty, &idx_ty);
                    self.annotations.set_resolved(*node_id, result.clone());
                    return result;
                }
                // CPython の物（`py_list` / `py_dict`）から読む要素は、読むたびに宣言の型と照合する（2-12）
                let python_read = matches!(obj_ty, InferredType::PyList(_) | InferredType::PyDict(..));
                let result = match obj_ty {
                    InferredType::ListOf(elem)
                    | InferredType::FixedListOf(elem)
                    | InferredType::ListLikeOf(elem)
                    | InferredType::PyList(elem) => *elem,
                    InferredType::PyDict(_, val) => *val,
                    InferredType::PySet(_) => InferredType::Unresolved,
                    // ⚠⚠ `set` は**添字アクセスできない**（実測: `'set' object is not
                    //    subscriptable`）。以前はここで要素型を返していたので、
                    //    **必ず実行時エラーになるコードに嘘の型**を与えていた。
                    //    診断は `check_subscript_index` が出す。
                    InferredType::Set
                    | InferredType::SetOf(_)
                    | InferredType::FrozenSet
                    | InferredType::FrozenSetOf(_) => InferredType::Unresolved,
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
                    // バイト列の添字は 0〜255 の `int`（3-1）
                    InferredType::Bytes => {
                        if matches!(idx_ty, InferredType::Int) { InferredType::Int } else { InferredType::Unresolved }
                    }
                    InferredType::Str => {
                        // 文字列の添字は文字列を返す
                        if matches!(idx_ty, InferredType::Int) { InferredType::Str } else { InferredType::Unresolved }
                    }
                    _ => InferredType::Unresolved,
                };
                // ── AST 型解決層（#16）── 添字アクセスの**要素結果型**を焼く。
                self.annotations.set_resolved(*node_id, result.clone());
                if python_read {
                    if let Some(check) = result.python_read_check() {
                        self.annotations.set_py_read(*node_id, check);
                    }
                }
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
            Expr::IsType { expr, type_name, node_id, .. } => {
                self.walk(expr);
                // 型名の妥当性（存在するか・`is` で判定できる型か）。⚠ 以前は `match` の腕でしか
                // 見ておらず、`if x is NoSuch:` が黙って通っていた（フェーズ10 10-10）。
                self.check_guard_type_exists(type_name);
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
                mut_targets,
                iter,
                body,
                return_type,
                comprehension,
            } => {
                let iter_ty = self.infer(iter);
                // ⚠ 反復できない型を弾く（タスク 7.1・検体 `K3`）。文と同じ検査を通す。
                self.check_iterable(&iter_ty, None);
                // ループ変数を要素型で宣言する（`Stmt::For` と同じ扱い）。
                // ここが抜けていたため、for **式**の本体では変数が未宣言＝`Unresolved` になり、
                // 本体の演算に型特化が効かなかった。
                let elem_ty = self.iter_element_type(&iter_ty);
                self.note_python_iteration(iter, &iter_ty, &elem_ty);
                // ⚠⚠ `for` **式**も `for` 文と同じ束縛を作るので、B4 の規則を**同じく適用**する。
                //    以前は `mutable: true` 決め打ちで、内包表記の脱糖先もここを通るため
                //    文側だけ直しても穴が残っていた（実測）。
                // `for mut x in ..`（D-11・2-10）は可変の束縛（要素を写す）
                let target_mut = *mut_targets || self.path_is_mutable(iter).unwrap_or(false);
                // `Stmt::For` と同じ（3-1）: ループ変数が「誰からも書き換わらない値」か
                let iter_proven = !*mut_targets && self.proven_immutable_path(iter);
                let mut_targets = *mut_targets;
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
                let yields = self.with_loop_expr_yielding(ann, |c| {
                    if *comprehension {
                        c.state.begin_comprehension();
                    }
                    c.push_scope();
                    c.for_var_proof.push(targets.iter().map(|t| (t.clone(), iter_proven)).collect());
                    for (target, ty) in targets.iter().zip(target_tys) {
                        // 規則 1: 外側に同名があれば再束縛（`Stmt::For` と同じ形）。
                        if target != "_" && c.already_declared(target) {
                            c.report_error(StaticTypeError {
                                kind: TypeErrorKind::VariableRedeclaration {
                                    name: target.clone(),
                                },
                                span: None,
                            });
                        }
                        // Python の物を受けるループ変数は `for mut` と書く（D-11・`Stmt::For` と同じ）
                        if !mut_targets && target != "_" {
                            c.require_mut_for_python_object(target, "for", &ty);
                        }
                        // 規則 3: 反復対象の属性を継ぐ（一時値は `let`）。
                        c.note_binding(target, &ty);
                        c.declare(target.clone(), ty, target_mut);
                    }
                    c.check_stmts(body);
                    c.for_var_proof.pop();
                    c.pop_scope();
                    if *comprehension {
                        c.state.end_comprehension()
                    } else {
                        Vec::new()
                    }
                });
                // 内包表記の型は要素の式の型から（known_bugs.md の #15・`Expr::ForExpr` の `comprehension`）
                if *comprehension && !yields.is_empty() {
                    return InferredType::ListOf(Box::new(Self::join_elem_types(yields)));
                }
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

// stmt/check/assign.rs — 属性と添字への代入の検査。

use super::*;

impl TypeChecker {
    /// 属性/添字への代入 (`obj.attr = v` / `a[i] = v` と複合代入版) を型検査する。
    /// 添字代入のルートが不変変数ならエラー、不変フィールドへの代入もエラーにする。
    ///
    /// `compound_op` が `None` のとき（＝通常代入）、フィールドの**宣言型**と
    /// 代入値の型を突き合わせる（0-2）。`Some(op)` のとき（＝複合代入）は
    /// [`TypeChecker::check_compound_assign`] の二段検査へ回す（D-8・タスク 5.1）。
    pub(super) fn check_attr_assign(&mut self, target: &Expr, value: &Expr, compound_op: Option<&BinOp>) {
        // ⚠⚠ **`let` 束縛への書き込みは 3 経路あるのに 1 つだけ実行時だった**（タスク 7.2・検体 `M5`）。
        //
        // | 書き方 | 以前 |
        // |---|---|
        // | `xs.append(2)` | 静的エラー（`check_mutating_method_receiver`） |
        // | `xs[0] = 2` | 静的エラー（ここ） |
        // | `c.n = 2` | **実行時エラー** ⛔ |
        //
        // 判定はどれも「**パスの根の可変性**」で同じ（規則 1: 要素・フィールドは根の属性を継ぐ）。
        // ⇒ 添字だけでなく**属性**も同じ検査に通す。
        // ⚠ `check_immutable_field_assign` は「**フィールド**が `let` 宣言か」を見る**別の検査**。
        //    こちらは「**束縛**が `let` か」。混同しないこと。
        //
        // ⚠⚠ **根が「値の束縛」のときだけ**に限ること。`scan_examples` が実測で教えた偽陽性:
        //
        // | 書き方 | 根 | なぜ弾いてはいけないか |
        // |---|---|---|
        // | `Counter.total = 100` | クラス名（`TypeValOf`） | `static mut` は**クラス側が**可変。束縛の可変性は無関係 |
        // | `m.x = 1` | import したモジュール | モジュールは `mutable: false` で束縛されている |
        //
        // これらは「不変な変数の中身を書き換えている」のではなく、**名前空間のメンバーへの代入**。
        if matches!(target, Expr::Subscript { .. } | Expr::Attr { .. }) {
            if let Some(name) = Self::path_root_ident(target) {
                if let Some(info) = self.lookup(name) {
                    // ⚠⚠ **`Unresolved` / `Any` も除外する。** `compare_wasm_frontend` が
                    //    `examples/interop/py_classvar.ar` で**エディタだけが出す偽陽性**を
                    //    検出した: `editor` ビルドは import 先を読まないのでモジュールの
                    //    束縛が `PyNamespace` ではなく `Unresolved` になり、
                    //    「不変な変数のフィールドへの代入」に見えてしまう。
                    //    ⇒ 根が何か判らないときは判断しない（根本原因① の教訓）。
                    let is_value_binding = !matches!(
                        info.ty,
                        InferredType::TypeValOf(_)
                            | InferredType::Namespace(..)
                            | InferredType::PyNamespace(..)
                            | InferredType::Unresolved
                            | InferredType::Any
                    );
                    // ⚠ 見るのは**中身を書き換えられるか**（付け替えではない）。`from m import X` の
                    //   `X` は付け替えられないが、元が `mut` なら中身は書き換えられる。
                    if is_value_binding && !info.contents_mutable {
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
        // 代入の左辺の属性は「読み」ではない（`attr_is_assign_target` の doc）。複合代入（`o.x += v`）は先に読むので
        // 読みとして検査する（CPython も `getattr` してから `__setattr__`）。
        self.attr_is_assign_target = compound_op.is_none() && matches!(target, Expr::Attr { .. });
        let target_ty = self.infer(target);
        self.attr_is_assign_target = false;
        let value_ty = self.infer(value);
        // 値の照合で出る誤りは**代入する値**を指す（diagnostic_position_plan.md の 3-3）。値が錨を持たない
        // （リテラル）ときは代入先の属性名・添字の対象。⚠ 上の不変性の誤りは書き込む先の誤りなので文の位置のまま。
        let anchor = value.anchor_span().or_else(|| target.anchor_span()).cloned();
        self.with_anchor(anchor, |c| c.check_attr_assign_value(target, compound_op, target_ty, value_ty));
    }

    /// [`Self::check_attr_assign`] の後半: 代入する値の型（`value_ty`）を代入先（`target_ty`）と照合する。
    fn check_attr_assign_value(
        &mut self,
        target: &Expr,
        compound_op: Option<&BinOp>,
        target_ty: InferredType,
        value_ty: InferredType,
    ) {
        // CPython の物の要素・属性へ Arrow の list などをキャストせずに入れない（`df["c"] = [1, 2]`・`df.columns = [..]`・D-12・2-11）
        if let Expr::Subscript { object, .. } | Expr::Attr { object, .. } = target {
            let recv = self.infer_quietly(object);
            if self.is_python_value(&recv) {
                self.check_no_arrow_collection_to_python(&value_ty, "assignment into a Python object".to_string());
            }
        }
        // ⚠⚠ **添字代入 `xs[i] = v` / `d[k] = v`**（タスク 5.2b・検体 `L5`）。
        //    以前は完全に無検査で、`list[int]` に `str` が、`dict[str,int]` に
        //    `int` キーや `str` 値が**黙って入っていた**（実測: `['s', 2]` /
        //    `{'a': 1, 1: 2}`）。
        //    ⚠ `infer(target)` が返すのは**容器の要素型 / 値型**なので、そのまま期待型に
        //      使える（添字そのものの型は `infer` の中で検査済み）。
        if let Expr::Subscript { object, index, .. } = target {
            // ⚠⚠ **スライス代入 `xs[a:b] = rhs` は別の義務**（タスク 5.2b では扱わない）。
            //    右辺は「要素」ではなく**要素の並び**で、実測では list / tuple / str の
            //    どれでも受け付ける（`collection.ar` が `(20, 30)` と `"abc"` を渡している）。
            //    要素型と突き合わせると**偽エラー**になる（実際に落ちた）。
            let index_is_slice = matches!(index.as_ref(), Expr::Slice { .. });
            if compound_op.is_none() && !index_is_slice {
                let container = self.infer_container_desc(object);
                self.check_subscript_assign(&value_ty, &target_ty, &container);
            }
            // ⚠ 複合代入 `xs[i] += v` は二段検査（D-8）の対象だが、添字の左辺は
            //    まだ通していない（タスク 5.1 は変数とフィールドだけ）。
            return;
        }
        // ⚠⚠ **trait 修飾フィールドへの代入 `o::T.f = v`**（タスク 5.2d・検体 `F3`）。
        //    以前は `infer` が `Unresolved` を返していたので**丸ごと無検査**で、
        //    A-3 で入れた実行時検査（`store_field`）だけが止めていた。
        if let Expr::TraitAccess { trait_name, attr, .. } = target {
            if compound_op.is_none() {
                let ctx = format!("field `{attr}` of trait `{trait_name}`");
                self.check_trait_field_assign(&value_ty, &target_ty, attr, trait_name, &ctx);
            }
            return;
        }
        let Expr::Attr { object, attr, span, attr_span, .. } = target else {
            return;
        };
        // 属性の誤りは**属性名**を指す（`.` ではない・diagnostic_position_plan.md の 3-5）。合成した属性参照（位置 0）は `.`。
        let span = if attr_span.line != 0 { attr_span } else { span };
        // ⚠ import したモジュールの `mut` のグローバル変数への代入（`g.MK = v`）は、その変数の型で検査する
        //   （ふつうの変数の付け替えと同じ `VarTypeMismatch`）。
        if let InferredType::Namespace(_, _, vars) = self.infer_quietly(object) {
            if compound_op.is_none()
                && matches!(vars.attrs.get(attr.as_str()), Some(crate::type_check::types::VarAttr::Mut))
                && !matches!(target_ty, InferredType::Unresolved | InferredType::Any)
            {
                let ctx = format!("variable `{}.{attr}`", vars.name);
                if !self.check_expected(&value_ty, &target_ty, false, &ctx) {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::VarTypeMismatch {
                            name: format!("{}.{attr}", vars.name),
                            expected: target_ty,
                            got: value_ty,
                        },
                        span: None,
                    });
                }
            }
            return;
        }
        // `import[py]` のモジュールの大域への代入（`gm.count = 5`・pandas_import_plan.md の 2-9）: Python ではどの大域も
        // 外から書き換えられる。型スタブに型のある大域変数（`count: int`）はその型で検査する（上の `mut` と同じ）。
        // ⚠ 関数・クラスの付け替え（モンキーパッチ）と型の分からない名前は検査しない（型スタブに値の型が無い）
        if let InferredType::PyNamespace(..) = self.infer_quietly(object) {
            if compound_op.is_none()
                && !matches!(
                    target_ty,
                    InferredType::Unresolved | InferredType::Any | InferredType::Function { .. } | InferredType::TypeValOf(_)
                )
            {
                let module = match object.as_ref() {
                    Expr::Ident { name, .. } => name.clone(),
                    _ => "module".to_string(),
                };
                let ctx = format!("variable `{module}.{attr}`");
                if !self.check_expected(&value_ty, &target_ty, false, &ctx) {
                    self.report_error(StaticTypeError {
                        kind: TypeErrorKind::VarTypeMismatch {
                            name: format!("{module}.{attr}"),
                            expected: target_ty,
                            got: value_ty,
                        },
                        span: Some(span.clone()),
                    });
                }
            }
            return;
        }
        // レシーバのクラス名。**推論を伴わない**スコープ引きで求める（`self` も
        // `NamedInstance(現在のクラス)` として束縛されているので同じ経路で引ける）。
        // 識別子以外のレシーバ（`f().x = v` など）は保守的に検査しない。
        // ⚠ `GenericInstance{Box,[int]}` も取りこぼさないこと（A-2）。`NamedInstance` だけを
        //    見ていると、テンプレートクラスのインスタンスに対する検査が**黙って消える**。
        let (class_name, subst) = match object.as_ref() {
            Expr::Ident { name, .. } => match self.receiver_class(name) {
                Some(pair) => pair,
                None => return,
            },
            _ => return,
        };
        // `__setattr__` を上書きした Python のクラスへの代入は、値を上書きの仮引数が受ける（宣言型と突き合わせない・#10）。
        if self.class_has_setattr_hook(&class_name) {
            return;
        }
        // 期待型を決める。
        //
        // ⚠⚠ `infer_attr` が引く `registry.class_field_details` は**そのクラス自身が
        //    宣言したフィールドしか持たない**。trait から継承したフィールド（`w.hp`）は
        //    **別のテーブル `trait_field_details`** に入っているので `Unresolved` になり、
        //    検査が素通りしていた（起票されたバグ報告が trait を挙げていたのはこの形）。
        //    ⚠ `collect_class_field_details` も `class_field_details` しか見ないので
        //      これだけでは足りない。基底 trait のテーブルを明示的に引く。
        // クラス側のディスクリプタ（タスク 1-12）。インスタンス経由の代入はデータディスクリプタなら
        // `__set__(obj, v)` の `v`、クラス経由（`C.x = d`）と非データディスクリプタはメンバーそのものを置き換える。
        // ⚠ `target_ty`（`infer_attr` の戻り値）は読んだ値の型（`__get__` の戻り値）なので、そのままでは使えない。
        let descriptor_expected = self.class_descriptor(&class_name, attr).map(|d| {
            let via_class = match object.as_ref() {
                Expr::Ident { name, .. } => match self.lookup(name) {
                    Some(info) => matches!(info.ty, InferredType::TypeValOf(_)),
                    None => true, // 変数として引けないクラス名（`receiver_class` の doc）
                },
                _ => false,
            };
            if d.is_data && !via_class {
                d.write.unwrap_or(InferredType::Unresolved)
            } else {
                d.declared
            }
        });
        // `__set__` の `value` に型が無い（`Any`）なら検査しない（下の宣言型へ落とすとディスクリプタのクラスと比べてしまう）。
        if matches!(descriptor_expected, Some(InferredType::Unresolved | InferredType::Any)) {
            return;
        }
        let expected = match descriptor_expected.unwrap_or(target_ty) {
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
        // ⚠⚠ **代入する値の側に型変数が残っているときも同じ**（タスク 2-8 段階 2）。
        //    宣言型の `T` は置換表で `unknown` に写るが、仮引数 `items: list[T]` の型には `T` が
        //    残るので、自動 `__init__` の `self.items = items` が「`list[unknown]` に `list[T]`」
        //    という偽陽性になっていた（実測）。型変数を含む代入は、単相化した具体クラス
        //    （`Tagged[int]`）の側で具体型どうしとして検査される。
        if self.mentions_type_param(&expected) || self.mentions_type_param(&value_ty) {
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
            span: None,
        });
    }

    /// 属性代入のレシーバ（識別子）が**どのクラスか**を求める（タスク 5.3）。
    ///
    /// 2 通りある:
    /// - **インスタンス**（`c.n = v`）… 変数として引ける。型は `NamedInstance` /
    ///   `GenericInstance`。
    /// - **クラス名そのもの**（`C.n = v`・`static mut` への代入）… 変数として引けるときは
    ///   `TypeValOf(NamedInstance(C))`、引けないときはレジストリのクラス名。
    ///
    /// ⚠⚠ **後者を扱っていなかったので `static mut` への代入が丸ごと無検査だった**
    /// （検体 `F4`。実測: `static mut n: int` に `C.n = "s"` が黙って通り `s` を出す）。
    /// インスタンス経由（`c.n = "s"`）は**同じフィールドなのに弾けていた**という
    /// 非対称な穴で、`class_and_subst` が `TypeValOf` を知らないのが原因。
    fn receiver_class(
        &self,
        name: &str,
    ) -> Option<(String, std::collections::HashMap<String, InferredType>)> {
        if let Some(info) = self.lookup(name) {
            let ty = info.ty.clone();
            // クラス名を値として持つ変数（`let K = C` など）もここを通る。
            let inner = match ty {
                InferredType::TypeValOf(inner) => *inner,
                other => other,
            };
            return self.class_and_subst(&inner);
        }
        // 変数として引けないクラス名（`C.n = v` の `C`）。
        if self.registry.is_known_class(name) {
            return self.class_and_subst(&InferredType::NamedInstance(name.to_string()));
        }
        None
    }

    /// trait 修飾フィールドへの代入 `o::T.f = v` を宣言型と照合する（タスク 5.2d・検体 `F3`）。
    fn check_trait_field_assign(
        &mut self,
        value_ty: &InferredType,
        expected: &InferredType,
        attr: &str,
        trait_name: &str,
        ctx: &str,
    ) {
        if matches!(
            expected,
            InferredType::Unresolved | InferredType::Any | InferredType::Never
        ) {
            return;
        }
        // ⚠⚠ **テンプレート trait の型変数は照合できない。** `trait Holder[T]: mut item: T` を
        //    `class IntBox(Holder[int])` が実装しているとき、期待型は `T` のままで
        //    具体型引数（`int`）は**基底名 `Holder` だけでは判らない**。
        //    ⚠ `mentions_type_param` はここでは効かない。あれは「**いま見えている**
        //      型変数」を見るもので、`IntBox` の中では `Holder` の `T` は見えていない
        //      （実測: `IntBox(5)` が `declared 'T' but got 'int'` で落ちた）。
        //    ⇒ その trait の**テンプレート引数名**と突き合わせる。
        //    `trait_conformance.ar` が「型変数を含む要求は照合を見送る」と明記している。
        if self.mentions_type_param(expected) || self.mentions_trait_param(trait_name, expected) {
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        if self.check_expected(value_ty, expected, false, ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::FieldTypeMismatch {
                field_name: attr.to_string(),
                class_name: trait_name.to_string(),
                expected: expected.clone(),
                got: value_ty.clone(),
            },
            span: None,
        });
    }

    /// 添字代入 `xs[i] = v` の**値**を容器の要素型と照合する（タスク 5.2b・検体 `L5`）。
    ///
    /// `expected` は `infer(target)` が返した要素型 / 値型。判定材料が無い型
    /// （`Any`・`Unresolved`・型変数）は照合しない。
    fn check_subscript_assign(
        &mut self,
        value_ty: &InferredType,
        expected: &InferredType,
        container: &str,
    ) {
        if matches!(
            expected,
            InferredType::Unresolved | InferredType::Any | InferredType::Never
        ) {
            return;
        }
        // ⚠ テンプレート型変数は具体型と突き合わせられない（`mentions_type_param`）。
        if self.mentions_type_param(expected) {
            return;
        }
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("element of {container}");
        if self.check_expected(value_ty, expected, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::SubscriptTypeMismatch {
                context: format!("element of {container}"),
                expected: expected.clone(),
                got: value_ty.clone(),
            },
            span: None,
        });
    }

    /// 添字代入のエラー文言に出す**容器の綴り**。
    ///
    /// ⚠ **推論をやり直さない**。`infer(target)` が既に容器を推論しているので、
    /// ここで `infer(object)` を呼ぶと診断が二重に出る（`check_attr_assign` の
    /// 冒頭コメントと同じ理由）。スコープ引きだけで済ませ、引けなければ綴りを諦める。
    fn infer_container_desc(&self, object: &Expr) -> String {
        match object {
            Expr::Ident { name, .. } => match self.lookup(name) {
                Some(info) => format!("`{}`", info.ty),
                None => format!("`{name}`"),
            },
            _ => "the container".to_string(),
        }
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
}

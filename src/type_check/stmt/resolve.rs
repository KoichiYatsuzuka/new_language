// stmt/resolve.rs — モジュール型収集と型注釈の解決: collect_module_types / type_ann_to_inferred / resolve_declared_type。

use {
    crate::ast::{Expr, Stmt, TupleTarget},
    crate::type_check::errors::{StaticTypeError, TypeErrorKind},
    crate::type_check::types::{FnTypeParam, InferredType},
    crate::type_check::TypeChecker,
};

impl TypeChecker {
    /// import 先モジュールの本体を検査して**注釈だけ**を採取する（#16 段階 F）。
    ///
    /// 型検査は従来メインプログラムの文だけを走査しており、`Stmt::Import` の `body` は
    /// `collect_module_types`（署名を読むだけ）しか通らなかった。そのため
    /// **import 先の関数本体には注釈が付かず**、同じコードでもメイン側より最適化が効かなかった
    /// （実測: メインの `v.x*v.x+v.y*v.y` は `FBIN_SS` になるのに、import 先の同じ式は `BIN` のまま）。
    ///
    /// **診断は捨てる**。モジュール自身の型エラーは、そのモジュールを直接実行/`--compile` した
    /// ときに報告されるべきもので、ここで出すと import 側に二重に出てしまう。
    /// 採取したいのは注釈テーブルへの書き込み（副作用）だけ。
    ///
    /// スコープは push/pop で隔離する。万一 import 先がメイン側の名前を拾っても、
    /// 影響は注釈が不正確になることだけで、VM の特化 op は実行時型が想定外なら汎用へ
    /// フォールバックするため結果は変わらない。
    pub(crate) fn annotate_module_body(
        &mut self,
        lang: &str,
        module: &[String],
        body: &[Stmt],
    ) {
        if !self
            .annotated_modules
            .insert((lang.to_string(), module.to_vec()))
        {
            return; // 収集済み（複数箇所からの import・入れ子 import）
        }
        let saved = std::mem::take(&mut self.diags);
        self.push_scope();
        self.check_stmts(body);
        self.pop_scope();
        self.diags = saved;
    }

    /// `for target in iter:` のターゲットに与える**要素型**を、イテラブルの型から求める。
    ///
    /// 反復の意味論は `Interpreter::make_for_iterator`
    /// （[control_flow.rs](../../interpreter/exec/control_flow.rs)）に合わせる:
    /// list / fixed_list / set は要素を、`str` は 1 文字ずつ（＝`str`）、タプルは各要素を返す。
    /// **`dict` は Arrow では反復不可**（実行時 `TypeError: object is not iterable`）なので扱わない。
    /// ジェネレータ・`__iter__` を持つインスタンス・Python オブジェクトは静的に要素型を決められない。
    ///
    /// 決められない場合は従来どおり `Unresolved`（＝下流の検査を抑制する）を返す。
    pub(crate) fn for_element_type(iter_ty: &InferredType) -> InferredType {
        match iter_ty {
            InferredType::ListOf(elem)
            | InferredType::FixedListOf(elem)
            | InferredType::ListLikeOf(elem)
            | InferredType::SetOf(elem) => (**elem).clone(),
            // タプルの反復は各要素を順に返すので、全要素が同型のときだけ確定できる。
            // 異種タプルは反復ごとに型が変わるため `Unresolved`。
            InferredType::Tuple(types) if !types.is_empty() && types.iter().all(|t| *t == types[0]) => {
                types[0].clone()
            }
            // 文字列の反復は 1 文字ずつの `str` を返す。
            InferredType::Str => InferredType::Str,
            _ => InferredType::Unresolved,
        }
    }

    /// モジュールの tl AST を浅くスキャンして「名前 → 型」マップを返す。
    pub(crate) fn collect_module_types(
        &self,
        body: &[Stmt],
    ) -> std::collections::HashMap<String, InferredType> {
        let mut map = std::collections::HashMap::new();
        for stmt in body {
            match stmt {
                Stmt::ClassDef { name, .. } => {
                    map.insert(
                        name.clone(),
                        InferredType::TypeValOf(Box::new(InferredType::NamedInstance(
                            name.clone(),
                        ))),
                    );
                }
                Stmt::FnDef { name, params, return_type, .. } => {
                    let ret = return_type
                        .as_deref()
                        .map(Self::type_ann_to_inferred)
                        .unwrap_or(InferredType::Unresolved);
                    // ★ Python の `*args` / `**kwargs` を持つ関数は**引数リストを公開しない**。
                    //
                    // `FnTypeParam` は「個数が固定の引数列」しか表せないので、
                    // `f(1, 2, 3)`（`*args` へ流れる）や `f(x=1)`（`**kwargs` へ流れる）を
                    // **正しく検査できない**。無理に検査すると
                    // `takes 1 argument(s) but 2 were given` / `has no parameter named 'x'` と
                    // **嘘のエラー**になる。⇒ `params: None` にして呼び出し検査を行わない。
                    // （`InferredType::Function { params: None, .. }` は戻り値型だけ返す既存の形。）
                    let has_open_arity = params.iter().any(|p| {
                        p.variadic || p.name == crate::ast::PY_KWARGS_PARAM
                    });
                    if has_open_arity {
                        map.insert(
                            name.clone(),
                            InferredType::Function {
                                params: None,
                                return_type: Box::new(ret),
                            },
                        );
                        continue;
                    }
                    let fn_params: Vec<FnTypeParam> = params
                        .iter()
                        .map(|p| FnTypeParam {
                            name: p.name.clone(),
                            mutable: p.mutable,
                            ty: p.type_ann
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Any),
                            // デフォルトを持つ仮引数は呼び出しで省略できる。これを落とすと
                            // `mod.f()` が `takes N argument(s)` で弾かれる（インタープリタは
                            // `evaluated_defaults` で正しく埋めるので、静的検査だけが嘘をつく形）。
                            has_default: p.default.is_some(),
                        })
                        .collect();
                    map.insert(
                        name.clone(),
                        InferredType::Function {
                            params: Some(fn_params),
                            return_type: Box::new(ret),
                        },
                    );
                }
                // Let/Const with a type annotation carry the type (used by Python stubs:
                // `let dumps: function->str` → Function { params: None, return_type: Str }).
                Stmt::Let(name, type_ann, _) | Stmt::Const(name, type_ann, _) => {
                    let ty = type_ann
                        .as_deref()
                        .and_then(InferredType::from_ann)
                        .unwrap_or(InferredType::Unresolved);
                    map.insert(name.clone(), ty);
                }
                Stmt::Mut(name, _, _) | Stmt::Static(name, _, _) => {
                    map.insert(name.clone(), InferredType::Unresolved);
                }
                Stmt::LetTuple { targets, .. } => {
                    for t in targets {
                        match t {
                            TupleTarget::Let(n) | TupleTarget::Mut(n) | TupleTarget::Bare(n) => {
                                map.insert(n.clone(), InferredType::Unresolved);
                            }
                            TupleTarget::Wildcard => {}
                        }
                    }
                }
                _ => {}
            }
        }
        map
    }

    /// プリミティブ型アノテーション文字列を対応する [`InferredType`] に変換する。未知の場合は `Unresolved`。
    pub(crate) fn type_ann_to_inferred(s: &str) -> InferredType {
        // C ABI 型（int32 等）は基底型（int/float）の別名として扱う
        let s = crate::ast::c_abi_base_type(s).unwrap_or(s);
        match s {
            "int" => InferredType::Int,
            "float" => InferredType::Float,
            "str" => InferredType::Str,
            "bool" => InferredType::Bool,
            "None" => InferredType::None,
            "Any" => InferredType::Any,
            _ => InferredType::Unresolved,
        }
    }

    // ---------------------------------------------------------------------------
    // Protocol helpers
    // ---------------------------------------------------------------------------

    /// 型アノテーション付き変数宣言の型を解決する。
    /// アノテーションがプロトコル名の場合、RHS 型の適合チェックを行い Protocol 型を返す。
    /// 型の中の `Never`（⊥）を `Any` へ開く（D-11 / U-6・タスク 4.1）。
    ///
    /// 空コレクションリテラルの要素型は `Never` だが、**注釈が無い束縛**ではそのまま
    /// 残すと「要素を足せない空リスト」になる。既定値として上端 `Any` へ開く。
    /// ⚠ 注釈がある場合は注釈が勝つので開く必要はない（`Never` は何へでもアップキャスト可）。
    fn open_never_to_any(ty: InferredType) -> InferredType {
        use InferredType as T;
        let rec = |t: &T| Box::new(Self::open_never_to_any(t.clone()));
        match &ty {
            T::Never => T::Any,
            T::ListOf(t) => T::ListOf(rec(t)),
            T::FixedListOf(t) => T::FixedListOf(rec(t)),
            T::ListLikeOf(t) => T::ListLikeOf(rec(t)),
            T::SetOf(t) => T::SetOf(rec(t)),
            T::DictOf(k, v) => T::DictOf(rec(k), rec(v)),
            T::Tuple(ts) => T::Tuple(
                ts.iter().map(|t| Self::open_never_to_any(t.clone())).collect(),
            ),
            _ => ty,
        }
    }

    /// コレクションリテラルの**判っている要素だけ**を宣言型と照合する
    /// （タスク 7.7・検体 `L17`）。
    ///
    /// ## 何を埋めるのか
    ///
    /// ```arrow
    /// mut zs: list = [1]
    /// let xs: list[str] = [1, zs[0]]   # 旧: 通る
    /// ```
    ///
    /// `zs[0]` が `Unresolved` なので `join_elem_types` が**リテラル全体**を
    /// `list[⊥不明]` にしてしまい、**判っている要素 `1` の照合まで消えて**いた。
    ///
    /// ## ⚠⚠ なぜ join を変えないのか
    ///
    /// 「判らない側が混ざったら全体が判らない」（タスク 5.5 で 3 箇所に揃えた規則）を
    /// 崩すと**下流で偽陽性**が出る:
    ///
    /// ```arrow
    /// let xs = [1, zs[0]]
    /// let s: str = xs[1]    # zs[0] が str かもしれないので弾いてはいけない
    /// ```
    ///
    /// ⇒ **推論する型は変えず**、期待型が判っている束縛地点でだけ要素を個別に見る。
    ///
    /// ## ⚠ 見るのは「リテラルの要素」だけ
    ///
    /// 要素式を再推論すると**診断が二重に出る**（`infer(value)` で既に推論済み）。
    /// ⇒ 型が式だけで決まり**診断を出しようがない**リテラル（`1` / `"s"` / `True` …）に
    /// 限って照合する。`zs[0]` のような式には触らない。
    ///
    /// ⚠ `join` が汚染されていない（＝要素型が判っている）ときは通常の
    /// `types_compatible` が既に見ているので、ここは動かさない。
    fn check_literal_elements(&mut self, ann: &str, rhs_ty: &InferredType, stmt: &Stmt) {
        use InferredType as T;
        // join が汚染されたときだけ働く。
        let poisoned_elem = match rhs_ty {
            T::ListOf(e) | T::SetOf(e) | T::FixedListOf(e) | T::ListLikeOf(e) => {
                matches!(**e, T::Unresolved)
            }
            _ => false,
        };
        if !poisoned_elem {
            return;
        }
        let Some(declared) = InferredType::from_ann(ann) else {
            return;
        };
        let expected = match &declared {
            T::ListOf(e) | T::SetOf(e) | T::FixedListOf(e) | T::ListLikeOf(e) => (**e).clone(),
            _ => return,
        };
        if matches!(expected, T::Unresolved | T::Any | T::Never) {
            return;
        }
        let value = match stmt {
            Stmt::Let(_, _, v) | Stmt::Const(_, _, v) | Stmt::Mut(_, _, v) => v,
            _ => return,
        };
        let elems = match value {
            Expr::List(es) | Expr::Set(es) => es,
            _ => return,
        };
        for e in elems {
            let Some(lit_ty) = Self::literal_type(e) else {
                continue;
            };
            // ⚠ 記憶域なので `Site::Other`（`__cast__` は挿入されない）。
            if self.types_compatible(&lit_ty, &expected, crate::type_check::type_utils::Site::Other)
            {
                continue;
            }
            // ⚠ 要素の不一致は添字代入（5.2b）と同じ種別に寄せる。文言が揃う。
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::SubscriptTypeMismatch {
                    context: format!("element of `{ann}`"),
                    expected: expected.clone(),
                    got: lit_ty,
                },
                span: None,
            });
        }
    }

    /// **式だけで型が決まり、推論しても診断を出さない**リテラルの型。
    ///
    /// ⚠ ここに「推論で診断が出うる式」を足してはいけない（二重報告になる）。
    fn literal_type(e: &Expr) -> Option<InferredType> {
        Some(match e {
            Expr::Int(_) => InferredType::Int,
            Expr::Float(_) => InferredType::Float,
            Expr::ImaginaryLit(_) => InferredType::Complex,
            Expr::Str(_) => InferredType::Str,
            Expr::Bool(_) => InferredType::Bool,
            Expr::None => InferredType::None,
            _ => return None,
        })
    }

    /// **注釈位置に素の容器型が書かれていたら弾く**（タスク 8.1・案 A）。
    ///
    /// ⚠⚠ **注釈位置だけで呼ぶこと。** 型テスト位置（`x is list` /
    /// `x mustbe list` / `case list:`）で呼んではいけない。そこでの素の容器名は
    /// 「list かどうか」を問う正しい用法で、`runtime_type_predicates.ar` が実演している。
    ///
    /// ⚠ 判定は [`InferredType::is_bare_container`] に集約してある。綴りで比較しないのは、
    /// C ABI 別名や alias 展開を経た綴りもここへ来るため。
    pub(crate) fn check_ann_not_bare(&mut self, ann: &str, what: &str) {
        let Some(ty) = InferredType::from_ann(ann) else { return };
        self.check_ann_names_exist(&ty, ann, what);
        if !ty.is_bare_container() {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::BareContainerAnnotation {
                ann: ty.to_string(),
                what: what.to_string(),
            },
            span: None,
        });
    }

    /// **注釈に現れる型名が実在するか**を検査する（タスク 8.5）。
    ///
    /// ⚠⚠ **呼ぶ位置が効く。** `is_type_param` は**スコープ状態**なので、
    /// テンプレート型変数を積んだ**後**で呼ばないと `fn f[T](x: T)` の `T` を
    /// 「存在しない型名」と誤判定する（計測で `T` が 17 件出た）。
    /// ⇒ `check_fn_def` / `check_gen_def` は `push_type_params` の後に呼んでいる。
    ///
    /// ⚠⚠ **外部言語のスタブは巻き込まれない。** py / C# の型名（`Figure` `Axes`
    /// `NoReturn` …）は import 先のモジュール本体の注釈に現れるが、
    /// [`Self::annotate_module_body`] が**そこで出た診断を捨てる**ので利用者には出ない。
    /// 実測: 例題 393 件で未知名は 43 箇所あり、**42 箇所が py スタブ由来**だった。
    /// ⇒ ここを「外部由来なら飛ばす」と書かないこと。モジュール境界の判断は
    ///   `annotate_module_body` が 1 箇所で持っている。
    fn check_ann_names_exist(&mut self, ty: &InferredType, ann: &str, what: &str) {
        // ⚠ import 先を読み込めていないなら、レジストリに無い＝存在しない とは言えない。
        if self.registry_incomplete {
            return;
        }
        let mut names = Vec::new();
        ty.collect_type_names(&mut names);
        for name in names {
            if self.type_name_exists(&name) {
                continue;
            }
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::UnknownTypeName {
                    name,
                    ann: ann.to_string(),
                    what: what.to_string(),
                },
                span: None,
            });
        }
    }

    /// その名前が型として実在するか（タスク 8.5）。
    ///
    /// ⚠ `is_known_class` はクラス・enum・`new_type` を含む。trait / protocol と
    /// **いま見えているテンプレート型変数**を足したものが「実在する型名」の全体。
    fn type_name_exists(&self, name: &str) -> bool {
        // ⚠ `generator` は実行時の型名で、クラスとしては登録されない（タスク 8.0）。
        if name == "generator" {
            return true;
        }
        self.registry.is_known_class(name)
            || self.registry.is_known_trait(name)
            || self.registry.is_protocol(name)
            || self.state.is_type_param(name)
    }

    pub(crate) fn resolve_declared_type(
        &mut self,
        type_ann: Option<&str>,
        rhs_ty: InferredType,
        var_name: &str,
        stmt: &Stmt,
    ) -> InferredType {
        let ann = match type_ann {
            // ⚠⚠ **注釈が無いときは `Never` を `Any` へ開く**（D-11 / U-6・タスク 4.1）。
            //    空リテラルの要素型は `Never`（⊥）だが、`Never` のまま束縛すると
            //    「**要素を足せない空リスト**」になってしまう（`Never` は値を持たない型）。
            //      let xs = []        # → list[Any]（何でも入れられる）
            //      mut xs: list[int] = []   # → list[int]（注釈が勝つ。`Never` はそこへ
            //                               #    アップキャストできるので通る）
            None => return Self::open_never_to_any(rhs_ty),
            Some(a) => a,
        };
        // ⚠ 素の容器型注釈は弾く（タスク 8.1・案 A）。
        self.check_ann_not_bare(ann, &format!("variable `{var_name}`"));
        // ⚠ リテラルの**判っている要素だけ**を期待型と照合する（タスク 7.7・検体 `L17`）。
        //    `join` が `Unresolved` に汚染されたときの取りこぼしを埋める。
        self.check_literal_elements(ann, &rhs_ty, stmt);
        // ⚠ protocol 名の注釈はここで打ち切って構わない。`check_protocol_conformance` が
        //    **右辺との照合も行う**（構造的適合検査）ため、下の整合性検査と二重にならない。
        if self.registry.is_protocol(ann) {
            let proto_name = ann.to_string();
            self.check_protocol_conformance(&rhs_ty, &proto_name, None, var_name);
            return InferredType::Protocol(proto_name);
        }
        // ── 通常の注釈（0-1）─────────────────────────────────────────────────
        //
        // ⚠⚠ ここは長らく **`rhs_ty` を返すだけ**で、注釈を照合も採用もしていなかった。
        //    `let a: int = "s"` が通るだけでなく、変数が **`str` として束縛**されていた
        //    （注釈は完全に捨てられていた）。Protocol / Intersection / Result の 3 つだけが
        //    上で特別扱いされており、それ以外の注釈は存在しないのと同じだった。
        let Some(declared) = InferredType::from_ann(ann) else {
            // 解釈できない注釈文字列。⚠ `from_ann` は失敗を `None` で返すので、
            //    ここで `rhs_ty` に倒さないと「型が無い」ではなく「型が違う」になる。
            return rhs_ty;
        };
        if matches!(declared, InferredType::Unresolved) {
            return rhs_ty;
        }
        // ── 妥当性検査（注釈自身が成り立つか）──────────────────────────────
        //
        // ⚠⚠ **ここで `return` しないこと。** 「妥当性検査（注釈が成り立つか）」と
        //    「整合性検査（右辺と適合するか）」は**別系統**で、両方を通す必要がある
        //    （再設計文書 D-14）。
        //    以前は `Intersection` / `Result` がこの位置で妥当性だけ見て **return** しており、
        //    **右辺と一切照合していなかった**:
        //      let v: Intersection[Alpha, Beta] = OnlyAlpha(1, 2)   # 片方だけ実装でも通った
        //      let r: Result[int, str] = 1.5                        # float でも通った
        //    仮引数・戻り値の位置では捕まっていたので、原因がこの関数だと特定できた。
        //    2 つのカテゴリを同じ分岐で扱うと「片方やれば済んだ気になる」ので分けてある。
        match &declared {
            InferredType::Intersection(types) => {
                let types_cloned = types.clone();
                self.check_intersection_members(&types_cloned, None);
            }
            InferredType::Result(ok_ty, err_ty) => {
                let (ok_ty, err_ty) = ((**ok_ty).clone(), (**err_ty).clone());
                self.validate_result_type(&ok_ty, &err_ty, None);
            }
            _ => {}
        }
        // ⚠ テンプレート型変数を含む注釈（`let x: T = …`）は照合も採用もしない。
        //    `from_ann` は大文字始まりの未知の識別子をクラス名にするので、型変数と
        //    実在のクラスが型の上では区別できない（`mentions_type_param` の doc）。
        if self.mentions_type_param(&declared) {
            return rhs_ty;
        }
        // ⚠⚠ **整合性検査は `types_compatible` を通す**（タスク 3.3）。
        //    この経路は以前 `type_matches` を直接呼んでいたため `resolve_protocols` が
        //    掛からず、`list[HasN]` の `HasN` が `NamedInstance` のまま**名前**で比較され、
        //    構造的に満たしているクラスを**誤って弾いていた**（タスク 1.3 で実測）。
        //    1.3 では手で `resolve_protocols` を 2 行足して直したが、3.3 で述語の中へ
        //    閉じ込めたので**新しい検査地点で同じ忘れ方ができない**。
        if !self.types_compatible(&rhs_ty, &declared, crate::type_check::type_utils::Site::Other) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::VarTypeMismatch {
                    name: var_name.to_string(),
                    expected: declared.clone(),
                    got: rhs_ty,
                },
                span: None,
            });
        }
        // ⚠ **注釈の型で束縛する**（照合を足すだけでは足りない）。
        //    これが右辺の推論型より情報量が多いケースが実在する:
        //      `mut xs: list[int] = []`      … 右辺は要素型なしの `List`
        //      `mut o: int|None = None`      … 右辺は `None` 型
        //      `let d: Drawable = circle`    … 右辺は具体型 `Circle`
        //    採用しないと下流の推論・オーバーロード解決・VM の型特化が右辺依存になる。
        declared
    }

}

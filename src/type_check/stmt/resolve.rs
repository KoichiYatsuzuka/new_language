// stmt/resolve.rs — モジュール型収集と型注釈の解決: collect_module_types / type_ann_to_inferred / resolve_declared_type。

use {
    crate::ast::{Stmt, TupleTarget},
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
    pub(crate) fn resolve_declared_type(
        &mut self,
        type_ann: Option<&str>,
        rhs_ty: InferredType,
        var_name: &str,
        _stmt: &Stmt,
    ) -> InferredType {
        let ann = match type_ann {
            None => return rhs_ty,
            Some(a) => a,
        };
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
        if !self.type_matches(&rhs_ty, &declared) {
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

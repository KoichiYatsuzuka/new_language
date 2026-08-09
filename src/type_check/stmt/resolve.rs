// stmt/resolve.rs — モジュール型収集と型注釈の解決: collect_module_types / type_ann_to_inferred / resolve_declared_type。

use {
    crate::ast::{Stmt, TupleTarget},
    crate::type_check::types::{FnTypeParam, InferredType},
    crate::type_check::TypeChecker,
};

impl TypeChecker {
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
                    let fn_params: Vec<FnTypeParam> = params
                        .iter()
                        .map(|p| FnTypeParam {
                            name: p.name.clone(),
                            mutable: p.mutable,
                            ty: p.type_ann
                                .as_deref()
                                .and_then(InferredType::from_ann)
                                .unwrap_or(InferredType::Any),
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
        // アノテーションがプロトコル名かチェック
        if self.registry.is_protocol(ann) {
            let proto_name = ann.to_string();
            self.check_protocol_conformance(&rhs_ty, &proto_name, None, var_name);
            return InferredType::Protocol(proto_name);
        }
        // アノテーションが交差型の場合、メンバー互換性をチェック
        if let Some(InferredType::Intersection(types)) = InferredType::from_ann(ann) {
            let types_cloned = types.clone();
            self.check_intersection_members(&types_cloned, None);
            return InferredType::Intersection(types);
        }
        // アノテーションが Result 型の場合、Ok 型と Err 型が同じでないかチェック
        if let Some(InferredType::Result(ok_ty, err_ty)) = InferredType::from_ann(ann) {
            self.validate_result_type(&ok_ty, &err_ty, None);
            return InferredType::Result(ok_ty, err_ty);
        }
        rhs_ty
    }

}

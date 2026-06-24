#![allow(dead_code)]

use std::collections::HashSet;

use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// `Result[T, E]` 型で T == E の場合に静的エラーを記録する。
    pub(super) fn validate_result_type(
        &mut self,
        ok_ty: &InferredType,
        err_ty: &InferredType,
        span: Option<crate::token::Span>,
    ) {
        if ok_ty == err_ty {
            self.report_error(super::errors::StaticTypeError {
                kind: super::errors::TypeErrorKind::ResultSameTypes {
                    ok_type: ok_ty.clone(),
                    err_type: err_ty.clone(),
                },
                span,
            });
        }
    }

    /// 引数型 `arg_ty` がパラメータの期待型 `expected` と互換性があるかを判定する。
    pub(super) fn type_matches(&self, arg_ty: &InferredType, expected: &InferredType) -> bool {
        if *arg_ty == InferredType::Unresolved {
            return true;
        }
        if *expected == InferredType::Any {
            return true;
        }
        if arg_ty == expected {
            return true;
        }
        // Protocol 型パラメータ: 適合チェックは別途実施するため、ここでは基本的に受け入れる
        if let InferredType::Protocol(proto_name) = expected {
            return matches!(
                arg_ty,
                InferredType::NamedInstance(_) | InferredType::Protocol(_) | InferredType::Any
            ) || {
                let _ = proto_name;
                false
            };
        }
        if *expected == InferredType::TypeVal {
            return matches!(arg_ty, InferredType::TypeValOf(_) | InferredType::TypeVal);
        }
        if let InferredType::TypeValOf(expected_inner) = expected {
            return match arg_ty {
                InferredType::TypeVal => true,
                InferredType::TypeValOf(arg_inner) => {
                    self.type_val_compatible(arg_inner, expected_inner)
                }
                _ => false,
            };
        }
        // list_like accepts both list and fixed_list
        let is_list_like = |t: &InferredType| matches!(
            t, InferredType::List | InferredType::ListOf(_)
              | InferredType::FixedList | InferredType::FixedListOf(_)
        );
        match (arg_ty, expected) {
            // list compatibility
            (InferredType::ListOf(_), InferredType::List) => return true,
            (InferredType::List, InferredType::ListOf(_)) => return true,
            (InferredType::ListOf(a), InferredType::ListOf(e)) => return self.type_matches(a, e),
            // fixed_list compatibility
            (InferredType::FixedListOf(_), InferredType::FixedList) => return true,
            (InferredType::FixedList, InferredType::FixedListOf(_)) => return true,
            (InferredType::FixedListOf(a), InferredType::FixedListOf(e)) => return self.type_matches(a, e),
            // list_like accepts list or fixed_list (with or without inner type)
            (a, InferredType::ListLike) if is_list_like(a) => return true,
            (a, InferredType::ListLikeOf(e)) if is_list_like(a) => {
                let a_inner = match a {
                    InferredType::ListOf(i) | InferredType::FixedListOf(i) => Some(i.as_ref()),
                    _ => None,
                };
                return a_inner.map_or(true, |ai| self.type_matches(ai, e));
            }
            (InferredType::SetOf(_), InferredType::Set) => return true,
            (InferredType::Set, InferredType::SetOf(_)) => return true,
            (InferredType::SetOf(a), InferredType::SetOf(e)) => return self.type_matches(a, e),
            (InferredType::DictOf(_, _), InferredType::Dict) => return true,
            (InferredType::Dict, InferredType::DictOf(_, _)) => return true,
            (InferredType::DictOf(ak, av), InferredType::DictOf(ek, ev)) => {
                return self.type_matches(ak, ek) && self.type_matches(av, ev);
            }
            _ => {}
        }
        if let InferredType::Union(union_types) = expected {
            return union_types.iter().any(|ut| self.type_matches(arg_ty, ut));
        }
        // Intersection型: arg_ty がすべての構成型にマッチする必要がある
        if let InferredType::Intersection(isect_types) = expected {
            return isect_types.iter().all(|it| self.type_matches(arg_ty, it));
        }
        // arg_ty が Intersection の場合: arg_ty のいずれかの構成型が expected にマッチすれば可
        if let InferredType::Intersection(isect_types) = arg_ty {
            return isect_types.iter().any(|it| self.type_matches(it, expected));
        }
        if let InferredType::NamedInstance(class_name) = arg_ty {
            let expected_name = expected.to_string();
            let cast_key = format!("__cast__[{}]", expected_name);
            if let Some(methods) = self.class_method_sigs.get(class_name.as_str()) {
                if methods.contains_key(&cast_key) {
                    return true;
                }
            }
            // Check class/trait inheritance: Duck(Flyable, Swimmable) satisfies Flyable
            if let InferredType::NamedInstance(_) = expected {
                if self.class_implements_trait(class_name, &expected_name) {
                    return true;
                }
            }
        }
        false
    }

    /// `arg_inner` が `expected_inner` と互換性のある型値かを判定する。
    pub(super) fn type_val_compatible(
        &self,
        arg_inner: &InferredType,
        expected_inner: &InferredType,
    ) -> bool {
        if arg_inner == expected_inner {
            return true;
        }

        let InferredType::NamedInstance(arg_name) = arg_inner else {
            return false;
        };

        let expected_name = expected_inner.to_string();

        let mut current = arg_name.clone();
        let mut seen = std::collections::HashSet::new();
        loop {
            let Some(orig_name) = self.new_type_originals.get(&current).cloned() else {
                break;
            };
            if !seen.insert(orig_name.clone()) {
                break;
            }
            if orig_name == expected_name {
                return true;
            }
            current = orig_name;
        }

        if let Some(bases) = self.class_bases.get(arg_name.as_str()) {
            return bases.contains(&expected_name);
        }

        false
    }

    /// `raise` できる型かを判定する。
    pub(super) fn is_error_instance_type(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::NamedInstance(class_name) => {
                self.class_implements_trait(class_name, "Error")
            }
            InferredType::Union(types) => types.iter().all(|t| self.is_error_instance_type(t)),
            _ => false,
        }
    }

    /// クラスが指定 trait を実装しているかを基底リストから確認する。
    pub(super) fn class_implements_trait(&self, class_name: &str, trait_name: &str) -> bool {
        let mut stack = vec![class_name.to_string()];
        let mut seen = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            let Some(bases) = self.class_bases.get(cur.as_str()) else {
                continue;
            };
            if bases.iter().any(|base| base == trait_name) {
                return true;
            }
            stack.extend(bases.iter().cloned());
        }
        false
    }
}

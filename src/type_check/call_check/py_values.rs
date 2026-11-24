// type_check/call_check/py_values.rs — Python の値との境界: `py_list` などのメソッドの型と、Arrow のコレクションを渡す誤り。

use super::*;

impl TypeChecker {
    /// CPython の物と型で分かるか（Python のモジュール・そのクラスとインスタンス・`py_list` など・D-12・2-11）。
    pub(in crate::type_check) fn is_python_value(&self, ty: &InferredType) -> bool {
        match ty {
            InferredType::PyNamespace(_) => true,
            InferredType::PyList(_) | InferredType::PyDict(..) | InferredType::PySet(_) => true,
            InferredType::NamedInstance(n) | InferredType::GenericInstance { name: n, .. } => self.registry.is_cpython_class(n),
            InferredType::TypeValOf(inner) => self.is_python_value(inner),
            _ => false,
        }
    }

    /// 型 `ty` が Arrow の list / dict / set を含むなら、写す先の Python の型（`list[int]` → `py_list[int]`・`tuple[list[int], int]`
    /// → `tuple[py_list[int], int]`）。含まなければ `None`。
    pub(crate) fn arrow_collection_as_python(ty: &InferredType) -> Option<InferredType> {
        use InferredType as T;
        fn conv(t: &T, changed: &mut bool) -> T {
            let mut rec = |t: &T| Box::new(conv(t, changed));
            let r = match t {
                T::ListOf(e) | T::FixedListOf(e) | T::ListLikeOf(e) => T::PyList(rec(e)),
                T::SetOf(e) => T::PySet(rec(e)),
                T::DictOf(k, v) => {
                    let k = rec(k);
                    T::PyDict(k, Box::new(conv(v, changed)))
                }
                T::List | T::FixedList | T::ListLike => T::PyList(Box::new(T::Any)),
                T::Set => T::PySet(Box::new(T::Any)),
                T::Dict => T::PyDict(Box::new(T::Any), Box::new(T::Any)),
                T::Tuple(ts) => return T::Tuple(ts.iter().map(|t| conv(t, changed)).collect()),
                T::Union(ts) => return T::Union(ts.iter().map(|t| conv(t, changed)).collect()),
                // 空のリテラル（`[]`）の要素は書いた人が決める
                T::Never => return T::Any,
                other => return other.clone(),
            };
            *changed = true;
            r
        }
        let mut changed = false;
        let out = conv(ty, &mut changed);
        changed.then_some(out)
    }

    /// Python へ渡す値の型 `ty` が Arrow の list / dict / set を含めば `ArrowCollectionToPython`（D-12・2-11）。リテラルも同じ
    /// （`np.array([1, 2] => py_list[int])` と書く）。
    pub(in crate::type_check) fn check_no_arrow_collection_to_python(&mut self, ty: &InferredType, context: String) {
        if let Some(convert_to) = Self::arrow_collection_as_python(ty) {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::ArrowCollectionToPython { got: ty.clone(), convert_to, context },
                span: None,
            });
        }
    }

    /// `py_list` / `py_dict` / `py_set`（中身は CPython の物・pandas_import_plan.md の D-12・2-7）のメソッドの戻り値の型。受け手が
    /// それらでなければ `None`、それらで型が決まらなければ `Some(None)`。CPython 3.12 の `list` / `dict` / `set` と同じ。
    /// ⚠ `keys()` / `values()` / `items()` は CPython のビュー（Python の dict_keys など）なので、反復の要素の型だけを付ける（`generator[K]`）。
    pub(super) fn py_collection_method_return(
        recv_ty: &InferredType,
        method: &str,
        args: &[(Option<String>, InferredType)],
    ) -> Option<Option<InferredType>> {
        use InferredType as T;
        let positional: Vec<&T> = args.iter().filter(|(k, _)| k.is_none()).map(|(_, t)| t).collect();
        let iter_of = |t: &T| Some(T::IteratorOf(Box::new(t.clone())));
        let r = match recv_ty {
            T::PyList(e) => match method {
                "append" | "extend" | "insert" | "remove" | "reverse" | "sort" | "clear" => Some(T::None),
                "pop" => Some((**e).clone()),
                "index" | "count" => Some(T::Int),
                "copy" => Some(recv_ty.clone()),
                "__iter__" => iter_of(e),
                _ => None,
            },
            T::PySet(e) => match method {
                "add" | "discard" | "remove" | "clear" | "update" | "difference_update" | "intersection_update"
                | "symmetric_difference_update" => Some(T::None),
                "pop" => Some((**e).clone()),
                "isdisjoint" | "issubset" | "issuperset" => Some(T::Bool),
                // 結果が受け手の部分集合になるもの。⚠ `union` / `symmetric_difference` は引数の要素が混ざる（引数は任意の
                //   イテラブル）ので、引数がすべて同じ型の `py_set` のときだけ型を付ける
                "copy" | "intersection" | "difference" => Some(recv_ty.clone()),
                "union" | "symmetric_difference" if args.iter().all(|(k, t)| k.is_none() && t == recv_ty) => {
                    Some(recv_ty.clone())
                }
                "__iter__" => iter_of(e),
                _ => None,
            },
            T::PyDict(k, v) => {
                let or_default = |d: &T| -> T {
                    match d {
                        _ if *d == **v || matches!(d, T::Never) || matches!(**v, T::Any) => (**v).clone(),
                        T::None => T::Union(vec![(**v).clone(), T::None]),
                        other => T::Union(vec![(**v).clone(), other.clone()]),
                    }
                };
                match method {
                    "keys" | "__iter__" => iter_of(k),
                    "values" => iter_of(v),
                    "items" => iter_of(&T::Tuple(vec![(**k).clone(), (**v).clone()])),
                    "get" | "setdefault" => match positional.as_slice() {
                        [_] => Some(or_default(&T::None)),
                        [_, d] => Some(or_default(d)),
                        _ => None,
                    },
                    "pop" => match positional.as_slice() {
                        [_] => Some((**v).clone()),
                        [_, d] => Some(or_default(d)),
                        _ => None,
                    },
                    "popitem" => Some(T::Tuple(vec![(**k).clone(), (**v).clone()])),
                    "copy" => Some(recv_ty.clone()),
                    "update" | "clear" => Some(T::None),
                    _ => None,
                }
            }
            _ => return None,
        };
        Some(r)
    }
}

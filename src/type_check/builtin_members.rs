// type_check/builtin_members.rs — 組み込みの値が持つ特殊メソッドと名前のあるメソッドの表（pandas_import_plan.md の 6-6）。
//
// protocol の構造的な適合（`list[int]` は `protocol HasLen` を満たすか）と、実行時の型テスト（`[1, 2] is HasLen`）が使う
// **唯一の表**。どの種類の値がどのメンバーを持つかは [`members_of`] だけが決め、静的な型（[`Member::sigs`]）と実行時の
// 名前（[`Member::name`]）はそこから引く。
//
// ⚠ CPython の型スタブ（typeshed）と同じく、`Sequence` / `Mapping` の抽象クラスが足すメソッド（`str` / `tuple` の
//   `__reversed__` / `count` / `index` など）も持つとする（CPython の `isinstance("ab", Sequence)` が真なのと同じ答え。
//   実物の `str` は `__reversed__` を持たないが、`reversed("ab")` は動く）。
// ⚠ 引数は位置だけで渡す（名前は空・CPython の組み込みと同じ）。protocol の照合は空の名前を比べない
//   （`method_sig_matches_protocol`）。

use crate::type_check::types::{FnSig, InferredType};

/// 組み込みの値の種類（静的な型 [`shape_of`] と実行時の値の両方から引く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltinKind {
    Int,
    Bool,
    Float,
    Complex,
    Str,
    Bytes,
    NoneValue,
    /// `list` / `fixed_list` / `list_like`（中身が Arrow の値）と `py_list`（中身が CPython の list）
    List,
    Tuple,
    /// `dict` と `py_dict`
    Dict,
    /// `set` と `py_set`
    Set,
    FrozenSet,
    /// ジェネレータ（`generator[T]`）
    Generator,
}

/// 組み込みの値のメンバー。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Member {
    Iter,
    Next,
    Len,
    GetItem,
    Contains,
    Reversed,
    Count,
    Index,
    Hash,
    IndexDunder,
    Int,
    Float,
    Complex,
    Bytes,
    Abs,
    Round,
    Keys,
    Values,
    Items,
    Get,
}

impl Member {
    /// Python のメンバーの名前（protocol の要求の名前と比べる）。
    pub(crate) fn name(self) -> &'static str {
        match self {
            Member::Iter => "__iter__",
            Member::Next => "__next__",
            Member::Len => "__len__",
            Member::GetItem => "__getitem__",
            Member::Contains => "__contains__",
            Member::Reversed => "__reversed__",
            Member::Count => "count",
            Member::Index => "index",
            Member::Hash => "__hash__",
            Member::IndexDunder => "__index__",
            Member::Int => "__int__",
            Member::Float => "__float__",
            Member::Complex => "__complex__",
            Member::Bytes => "__bytes__",
            Member::Abs => "__abs__",
            Member::Round => "__round__",
            Member::Keys => "keys",
            Member::Values => "values",
            Member::Items => "items",
            Member::Get => "get",
        }
    }

    /// `shape` の値のこのメンバーのシグネチャ（オーバーロードの並び・`self` を含まない）。
    pub(crate) fn sigs(self, s: &Shape) -> Vec<FnSig> {
        use InferredType as T;
        let sig = |params: Vec<InferredType>, required: usize, ret: InferredType| FnSig {
            param_mutable: vec![false; params.len()],
            params: params.into_iter().map(|t| (String::new(), Some(t))).collect(),
            required_count: required,
            return_type: Some(ret),
            variadic_type: None,
        };
        let iter = |e: &T| T::IteratorOf(Box::new(e.clone()));
        // `dict.keys()` などは Arrow の dict ならリスト、`py_dict` なら反復子（`builtin_collection_method_return` /
        // `py_collection_method_return` と同じ）
        let view = |e: T| if s.python { iter(&e) } else { T::ListOf(Box::new(e)) };
        match self {
            Member::Iter | Member::Reversed => vec![sig(vec![], 0, iter(&s.elem))],
            Member::Next => vec![sig(vec![], 0, s.elem.clone())],
            Member::Len | Member::Hash | Member::IndexDunder | Member::Int => vec![sig(vec![], 0, T::Int)],
            Member::Float => vec![sig(vec![], 0, T::Float)],
            Member::Complex => vec![sig(vec![], 0, T::Complex)],
            Member::Bytes => vec![sig(vec![], 0, T::Bytes)],
            Member::GetItem => vec![sig(vec![s.key.clone()], 1, s.value.clone())],
            // ⚠ 受ける値は何でもよい（CPython の `Container.__contains__(self, x: object)`。`str` の実物は `str` だけを
            //   受けるが、型スタブは `Sequence` として `object` を受けるとする）
            Member::Contains => vec![sig(vec![T::Any], 1, T::Bool)],
            Member::Count => vec![sig(vec![T::Any], 1, T::Int)],
            Member::Index => vec![sig(vec![T::Any, T::Int, T::Int], 1, T::Int)],
            Member::Abs => vec![sig(
                vec![],
                0,
                match s.kind {
                    BuiltinKind::Complex => T::Float,
                    BuiltinKind::Bool => T::Int,
                    _ => s.this.clone(),
                },
            )],
            // `round(x)` は int、`round(x, n)` は x の型（bool は int）
            Member::Round => {
                let with_digits = if s.kind == BuiltinKind::Float { T::Float } else { T::Int };
                vec![sig(vec![], 0, T::Int), sig(vec![T::Int], 1, with_digits)]
            }
            Member::Keys => vec![sig(vec![], 0, view(s.key.clone()))],
            Member::Values => vec![sig(vec![], 0, view(s.value.clone()))],
            Member::Items => vec![sig(vec![], 0, view(T::Tuple(vec![s.key.clone(), s.value.clone()])))],
            Member::Get => vec![
                sig(vec![s.key.clone()], 1, T::Union(vec![s.value.clone(), T::None])),
                sig(vec![s.key.clone(), s.value.clone()], 2, s.value.clone()),
            ],
        }
    }
}

/// 種類ごとのメンバー（**唯一の表**）。CPython 3.12 の組み込みの型と、型スタブの `Sequence` / `Mapping` の抽象クラスが
/// 足すメソッド。
pub(crate) fn members_of(kind: BuiltinKind) -> &'static [Member] {
    use Member as M;
    match kind {
        BuiltinKind::Int | BuiltinKind::Bool => &[M::Hash, M::IndexDunder, M::Int, M::Float, M::Abs, M::Round],
        BuiltinKind::Float => &[M::Hash, M::Int, M::Float, M::Abs, M::Round],
        BuiltinKind::Complex => &[M::Hash, M::Complex, M::Abs],
        BuiltinKind::Str => &[M::Iter, M::Len, M::GetItem, M::Contains, M::Reversed, M::Count, M::Index, M::Hash],
        BuiltinKind::Bytes => {
            &[M::Iter, M::Len, M::GetItem, M::Contains, M::Reversed, M::Count, M::Index, M::Hash, M::Bytes]
        }
        BuiltinKind::NoneValue => &[M::Hash],
        BuiltinKind::List => &[M::Iter, M::Len, M::GetItem, M::Contains, M::Reversed, M::Count, M::Index],
        BuiltinKind::Tuple => &[M::Iter, M::Len, M::GetItem, M::Contains, M::Reversed, M::Count, M::Index, M::Hash],
        BuiltinKind::Dict => {
            &[M::Iter, M::Len, M::GetItem, M::Contains, M::Reversed, M::Keys, M::Values, M::Items, M::Get]
        }
        BuiltinKind::Set => &[M::Iter, M::Len, M::Contains],
        BuiltinKind::FrozenSet => &[M::Iter, M::Len, M::Contains, M::Hash],
        BuiltinKind::Generator => &[M::Iter, M::Next],
    }
}

/// `kind` の値が名前 `name` のメンバーを持つか（実行時の型テスト `x is P` が使う）。
pub(crate) fn has_member(kind: BuiltinKind, name: &str) -> bool {
    members_of(kind).iter().any(|m| m.name() == name)
}

/// 静的な型の種類と、要素・キー・値の型（[`Member::sigs`] が使う）。
pub(crate) struct Shape {
    pub(crate) kind: BuiltinKind,
    /// 反復で出る型（dict はキー）
    elem: InferredType,
    /// `__getitem__` が受ける型（列は `int`・dict はキー）
    key: InferredType,
    /// `__getitem__` が返す型
    value: InferredType,
    /// 値そのものの型（`__abs__` の戻り値）
    this: InferredType,
    /// 中身が CPython の物か（`py_dict.keys()` は反復子）
    python: bool,
}

/// 組み込みの型なら、その種類と要素の型。クラス・protocol・関数などは `None`。
///
/// ⚠ `match` は網羅（`_` を書かない）。型の変種を足したら、ここで組み込みの値かを決めること。
pub(crate) fn shape_of(ty: &InferredType) -> Option<Shape> {
    use InferredType as T;
    let un = T::Unresolved;
    let mk = |kind: BuiltinKind, elem: &T, key: &T, value: &T, python: bool| Shape {
        kind,
        elem: elem.clone(),
        key: key.clone(),
        value: value.clone(),
        this: ty.clone(),
        python,
    };
    let scalar = |kind: BuiltinKind| mk(kind, &T::Unresolved, &T::Unresolved, &T::Unresolved, false);
    Some(match ty {
        T::Int => scalar(BuiltinKind::Int),
        T::Bool => scalar(BuiltinKind::Bool),
        T::Float => scalar(BuiltinKind::Float),
        T::Complex => scalar(BuiltinKind::Complex),
        T::None => scalar(BuiltinKind::NoneValue),
        T::Str => mk(BuiltinKind::Str, &T::Str, &T::Int, &T::Str, false),
        T::Bytes => mk(BuiltinKind::Bytes, &T::Int, &T::Int, &T::Int, false),
        T::ListOf(e) | T::FixedListOf(e) | T::ListLikeOf(e) => mk(BuiltinKind::List, e, &T::Int, e, false),
        T::List | T::FixedList | T::ListLike => mk(BuiltinKind::List, &un, &T::Int, &un, false),
        T::PyList(e) => mk(BuiltinKind::List, e, &T::Int, e, true),
        T::Tuple(ts) => {
            let e = tuple_element(ts);
            mk(BuiltinKind::Tuple, &e, &T::Int, &e, false)
        }
        T::TupleAny => mk(BuiltinKind::Tuple, &un, &T::Int, &un, false),
        T::DictOf(k, v) => mk(BuiltinKind::Dict, k, k, v, false),
        T::Dict => mk(BuiltinKind::Dict, &un, &un, &un, false),
        T::PyDict(k, v) => mk(BuiltinKind::Dict, k, k, v, true),
        T::SetOf(e) => mk(BuiltinKind::Set, e, &un, &un, false),
        T::Set => mk(BuiltinKind::Set, &un, &un, &un, false),
        T::PySet(e) => mk(BuiltinKind::Set, e, &un, &un, true),
        T::FrozenSetOf(e) => mk(BuiltinKind::FrozenSet, e, &un, &un, false),
        T::FrozenSet => mk(BuiltinKind::FrozenSet, &un, &un, &un, false),
        T::IteratorOf(e) => mk(BuiltinKind::Generator, e, &un, &un, false),
        // 組み込みの値ではない（クラス・protocol は自分のメンバーで照合する・型の分からない物は判定材料が無い）
        T::Undefined
        | T::Code
        | T::Meta(_)
        | T::Ptr(_)
        | T::TypeVal
        | T::TypeValOf(_)
        | T::SelfType
        | T::NamedInstance(_)
        | T::GenericInstance { .. }
        | T::Protocol(..)
        | T::Any
        | T::Union(_)
        | T::Result(..)
        | T::Intersection(_)
        | T::Namespace(..)
        | T::PyNamespace(..)
        | T::Unresolved
        | T::Never
        | T::Function { .. } => return None,
    })
}

/// タプルの反復で出る型: 要素の型の和（同じ型が並べばその型・空のタプルは `Never`）。
fn tuple_element(ts: &[InferredType]) -> InferredType {
    let mut distinct: Vec<InferredType> = Vec::new();
    for t in ts {
        if !distinct.contains(t) {
            distinct.push(t.clone());
        }
    }
    match distinct.len() {
        0 => InferredType::Never,
        1 => distinct.pop().expect("one element"),
        _ => InferredType::Union(distinct),
    }
}

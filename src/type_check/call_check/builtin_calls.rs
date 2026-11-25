// type_check/call_check/builtin_calls.rs — 組み込み関数・組み込みコレクションのメソッド・`Ptr` のメソッドの検査と戻り値の型。

use super::*;

impl TypeChecker {
    /// `len()` の引数が**大きさを持つ型**かを検査する（タスク 7.4）。
    ///
    /// ## 実測した実行時の規則
    ///
    /// | 大きさを持つ | 持たない |
    /// |---|---|
    /// | `list` ・ `fixed_list` ・ `list_like` ・ `str` ・ `dict` ・ `set` ・ `tuple` | `int` ・ `float` ・ `bool` ・ `None` |
    /// | `__len__` を持つクラス | `__len__` を持たないクラス（`object of type 'object' has no len()`） |
    ///
    /// ⚠ `InferredType` に「大きさを持つ型」は無いので、仮引数の型は `Any` にして
    /// **明らかに持たない型だけ**をここで弾く（7.1 と同じ「判っていて不可なら報告」）。
    /// ⚠ クラスは `__len__` を定義できるので素通しする。
    pub(super) fn check_len_argument(&mut self, arg_data: &[(Option<String>, InferredType)], args: &[CallArg]) {
        use InferredType as T;
        let [(None, got)] = arg_data else { return };
        let no_len = matches!(
            got,
            T::Int | T::Float | T::Complex | T::Bool | T::None | T::Undefined
        );
        if !no_len {
            return;
        }
        self.report_error_at_opt(Self::arg_expr(args, 0), StaticTypeError {
            kind: TypeErrorKind::NotSized { ty: got.clone() },
            span: None,
        });
    }

    /// **組み込みコレクションのメソッド引数**を要素型と照合する（タスク 5.2c・検体 `L14`）。
    ///
    /// ## 対象は「要素を 1 つ受け取るメソッド」だけ
    ///
    /// | レシーバ | メソッド | 引数 |
    /// |---|---|---|
    /// | `list` / `fixed_list` / `list_like` | `append` | 要素 |
    /// | `set` | `add` ・ `discard` ・ `remove` | 要素 |
    ///
    /// ⚠⚠ **実装側と必ず突き合わせること。** 実在するのは
    /// `src/interpreter/classes/method_call.rs`（list）・`set_methods.rs`（set）・
    /// `frozen_list_methods.rs`（fixed_list）に書かれているものだけで、
    /// `insert` / `extend` / `remove`（list）/ `index` / `count` / `get`（dict）は
    /// **存在しない**（実測: `AttributeError: 'list' object has no method 'insert'`）。
    /// 推測で足すと「実在しないメソッドの引数を検査する」死んだ枝になる。
    ///
    /// ⚠ `union` / `intersection` 等は**集合そのもの**を受け取るので対象外
    /// （要素型と突き合わせると偽エラーになる）。
    ///
    /// ⚠ 弾かなかった場合に何が起きるかは地点で違う（実測）:
    /// `list.append` と `set.add` は**黙って異型を入れる**（`[1, 's']` / `{1, 's'}`）、
    /// `set.discard` は黙って何もしない、`set.remove` は `KeyError`。
    /// どれも「要素型が守られない」ことに変わりはない。
    /// 組み込み**関数**の戻り値型（分かるものだけ）。`enumerate` / `zip` は引数から組み立て、
    /// 他は宣言ファイル `builtins.ars` の戻り値の型（`builtins::return_type`）。
    ///
    /// ⚠⚠ **`enumerate` / `zip` に型が付いていなかった**ので、
    /// `for i, c in enumerate(xs):` のループ変数が `Unresolved` 止まりになり、
    /// `str(i) + c` のような普通の式で型特化も検査も効かなかった。
    ///
    /// ⚠ 実行時の値は `Value::Generator` なので **`ListOf` ではなく `IteratorOf`** を返す。
    /// `ListOf` にすると `len()` や添字が静的に通ってしまい、実行時に落ちる
    /// （「反復すると T が出る」以上のことを約束しない型が要る）。
    ///
    /// ⚠ 判らない引数は `Any` にする（`Unresolved` は万能受容体になり検査を消す）。
    pub(super) fn builtin_fn_return(
        &self,
        name: &str,
        arg_data: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        use InferredType as T;
        // 要素型を引く。判らなければ `Any`。
        let elem = |t: &T| -> T {
            match self.iter_element_type(t) {
                T::Unresolved => T::Any,
                other => other,
            }
        };
        match name {
            // `enumerate(xs)` / `enumerate(xs, start)` → (index, 要素) の列。
            "enumerate" => {
                let (_, first) = arg_data.first()?;
                Some(T::IteratorOf(Box::new(T::Tuple(vec![T::Int, elem(first)]))))
            }
            // `zip(a, b, ...)` → 各列の要素を並べたタプルの列。
            "zip" => {
                if arg_data.is_empty() {
                    return None;
                }
                let ts: Vec<T> = arg_data.iter().map(|(_, t)| elem(t)).collect();
                Some(T::IteratorOf(Box::new(T::Tuple(ts))))
            }
            // 真偽を返す Python の組み込み（python_builtins_plan.md のフェーズ 2）。
            "isinstance" | "hasattr" | "issubclass" | "callable" | "all" | "any" => Some(T::Bool),
            // 集計（タスク 3-2）。⚠ 要素の型が分からないときは `None`（`Any` にすると下流が静的な誤りになる）。
            "sorted" | "reversed" | "min" | "max" | "sum" => {
                let positional: Vec<&T> =
                    arg_data.iter().filter(|(k, _)| k.is_none()).map(|(_, t)| t).collect();
                let has_kw = |n: &str| arg_data.iter().any(|(k, _)| k.as_deref() == Some(n));
                let known_elem = |t: &T| match self.iter_element_type(t) {
                    T::Unresolved | T::Any => None,
                    e => Some(e),
                };
                match (name, positional.as_slice()) {
                    ("sorted", [t]) => known_elem(t).map(|e| T::ListOf(Box::new(e))),
                    ("reversed", [t]) => known_elem(t).map(|e| T::IteratorOf(Box::new(e))),
                    // `default=` は別の型を返しうるので付けない。
                    ("min" | "max", [t]) if !has_kw("default") => known_elem(t),
                    ("min" | "max", [first, rest @ ..])
                        if !rest.is_empty() && rest.iter().all(|t| t == first) && !matches!(first, T::Unresolved | T::Any) =>
                    {
                        Some((*first).clone())
                    }
                    ("sum", [t]) if !has_kw("start") => match known_elem(t) {
                        Some(T::Int) => Some(T::Int),
                        Some(T::Float) => Some(T::Float),
                        _ => None,
                    },
                    _ => None,
                }
            }
            // 変換・遅延の組み込み（タスク 3-3）。`map` は `f` の戻り値が分からないので付けない。
            "hash" | "ord" => Some(T::Int),
            "chr" | "hex" | "oct" | "bin" | "ascii" | "format" => Some(T::Str),
            "filter" | "iter" => {
                let src = match (name, arg_data) {
                    ("filter", [_, (None, t)]) | ("iter", [(None, t)]) => Some(t),
                    _ => None,
                };
                src.and_then(|t| match self.iter_element_type(t) {
                    T::Unresolved | T::Any => None,
                    e => Some(T::IteratorOf(Box::new(e))),
                })
            }
            "abs" => match arg_data {
                [(None, T::Int)] => Some(T::Int),
                [(None, T::Float | T::Complex)] => Some(T::Float),
                _ => None,
            },
            "sqrt" | "cbrt" => match arg_data {
                [(None, T::Int | T::Float | T::Bool)] => Some(T::Float),
                _ => None,
            },
            // 指数・対数。引数（`base=` のキーワードも含む）がすべて数値と分かるときだけ float。
            "exp" | "log" | "exp2" | "exp10" | "log2" | "log10"
                if !arg_data.is_empty() && arg_data.iter().all(|(_, t)| matches!(t, T::Int | T::Float | T::Bool)) =>
            {
                Some(T::Float)
            }
            "round" => match arg_data {
                [(None, T::Int | T::Float)] => Some(T::Int),
                [(None, t @ (T::Int | T::Float)), (_, T::Int)] => Some(t.clone()),
                _ => None,
            },
            "divmod" => match arg_data {
                [(None, T::Int), (None, T::Int)] => Some(T::Tuple(vec![T::Int, T::Int])),
                [(None, T::Int | T::Float), (None, T::Int | T::Float)] => Some(T::Tuple(vec![T::Float, T::Float])),
                _ => None,
            },
            "pow" => match arg_data {
                [(None, T::Int), (None, T::Int), (None, T::Int)] => Some(T::Int),
                _ => None,
            },
            "setattr" => Some(T::None),
            // `type(x)` は x の型の**型値**（タスク 2-2）。`type(x)(...)` が x と同じ型になり、`__name__` が引ける。
            // ⚠ クラスとプリミティブだけ。判らない型・3 引数の形は `None`（嘘の型を返さない）。
            "type" => match arg_data {
                [(None, t @ (T::NamedInstance(_) | T::Int | T::Float | T::Str | T::Bool | T::Complex))] => {
                    Some(T::TypeValOf(Box::new(t.clone())))
                }
                _ => None,
            },
            // `list()` は `[]` と同じ（要素型 `Never`）、`list(xs)` は要素の list（タスク 1-7）。
            // ⚠ 要素型が分からない（Python 由来の値など）ときは `None`（`list[Any]` にすると要素の操作が静的な誤りになる）。
            "list" => match arg_data {
                [] => Some(T::ListOf(Box::new(T::Never))),
                [(None, t)] => match self.iter_element_type(t) {
                    T::Unresolved => None,
                    e => Some(T::ListOf(Box::new(e))),
                },
                _ => None,
            },
            // `bytes(..)`（タスク 3-1）。どの形でも bytes（失敗は実行時の誤り）。
            "bytes" => Some(T::Bytes),
            // `frozenset()` / `frozenset(xs)`（タスク 3-1）。`list(..)` と同じ考え方。
            "frozenset" => match arg_data {
                [] => Some(T::FrozenSetOf(Box::new(T::Never))),
                [(None, t)] => match self.iter_element_type(t) {
                    T::Unresolved => None,
                    e => Some(T::FrozenSetOf(Box::new(e))),
                },
                _ => None,
            },
            // 他の組み込み関数は宣言（`src/built_in_stab/builtins.ars`）の戻り値の型
            // （`open` → `FileObject`・`repr` → `str` …）。宣言に型が無いものは `None`（従来どおり）。
            _ => crate::type_check::builtins::return_type(name),
        }
    }

    /// 組み込みコレクションのメソッドの**戻り値型**（分かるものだけ）。
    ///
    /// ⚠⚠ **以前は何も返していなかった**ので `d.keys()` が `Unresolved` になり、
    /// `let s: str = d.keys()` が**黙って通って**いた（実測）。`Unresolved` は下流で
    /// 万能受容体として振る舞うため、検査がそこで消える。
    ///
    /// ⚠ とくに `d.items()` に型が付くと `for k, v in d.items():` のループ変数へ
    /// **キー型・値型がそのまま届く**（`Tuple([K, V])` の位置対応）。付いていないと
    /// `Any` 止まりになり、`k + str(v)` のような普通の式が明示ダウンキャストを要求される。
    ///
    /// ⚠ 判らないものは `None`（従来どおり）。ここで嘘の型を返さないこと。
    pub(in crate::type_check) fn builtin_collection_method_return(
        recv_ty: &InferredType,
        method: &str,
        args: &[(Option<String>, InferredType)],
    ) -> Option<InferredType> {
        use InferredType as T;
        // 文字コードの変換（3-1）。⚠ ほかの `str` のメソッドはまだ型が無い（結果は `Unresolved`）。
        match (recv_ty, method) {
            (T::Str, "encode") => return Some(T::Bytes),
            (T::Bytes, "decode") => return Some(T::Str),
            // `Ptr[T]`（3-3）。できない組み合わせは `check_ptr_method` が報告する
            (T::Ptr(_), "release") => return Some(T::None),
            (T::Ptr(e), "to_list") => return e.list_elem().map(|t| T::ListOf(Box::new(t))),
            (T::Ptr(e), "to_bytes") if e.is_char() => return Some(T::Bytes),
            _ => {}
        }
        if let Some(r) = Self::py_collection_method_return(recv_ty, method, args) {
            return r;
        }
        // `x.__iter__()` は要素の列（python_builtins_plan.md のタスク 1-4）。辞書はキー、ジェネレータは自分自身。
        if method == "__iter__" {
            return match recv_ty {
                T::IteratorOf(_) => Some(recv_ty.clone()),
                T::DictOf(k, _) => Some(T::IteratorOf(k.clone())),
                T::ListOf(e) | T::FixedListOf(e) | T::ListLikeOf(e) | T::SetOf(e) | T::FrozenSetOf(e) => {
                    Some(T::IteratorOf(e.clone()))
                }
                T::Str => Some(T::IteratorOf(Box::new(T::Str))),
                _ => None,
            };
        }
        // `s.encode()` は bytes、bytes のメソッド（タスク 3-1・`classes/bytes_methods.rs`）。
        if matches!(recv_ty, T::Str) && method == "encode" {
            return Some(T::Bytes);
        }
        if matches!(recv_ty, T::Bytes) {
            return match method {
                "decode" | "hex" => Some(T::Str),
                "count" | "find" | "rfind" | "index" | "rindex" => Some(T::Int),
                "startswith" | "endswith" | "isalnum" | "isalpha" | "isascii" | "isdigit" | "islower" | "isspace"
                | "isupper" => Some(T::Bool),
                "replace" | "join" | "strip" | "lstrip" | "rstrip" | "upper" | "lower" => Some(T::Bytes),
                "split" => Some(T::ListOf(Box::new(T::Bytes))),
                "__iter__" => Some(T::IteratorOf(Box::new(T::Int))),
                _ => None,
            };
        }
        // set / frozenset の書き換えないメソッド（タスク 3-1）。結果が受け手の部分集合になるもの（`copy` /
        // `intersection` / `difference`）は受け手と同じ型。
        // ⚠ `union` / `symmetric_difference` は引数の要素が混ざる（引数は任意のイテラブル・個数も可変）ので
        //   返さない（嘘の型を返さない）。
        if let T::SetOf(_) | T::FrozenSetOf(_) | T::Set | T::FrozenSet = recv_ty {
            return match method {
                "copy" | "intersection" | "difference" => Some(recv_ty.clone()),
                "issubset" | "issuperset" | "isdisjoint" => Some(T::Bool),
                _ => None,
            };
        }
        // ジェネレータ（`generator[T]`・フェーズ10 10-7）。`next()` は要素 1 つ、`close()` は `None`。
        // ⚠ 以前は結果に型が無く、`let v: str = g.next()` が通っていた（`for` と同じ穴）。
        if let T::IteratorOf(elem) = recv_ty {
            return match method {
                "next" => Some((**elem).clone()),
                "close" => Some(T::None),
                _ => None,
            };
        }
        let (k, v) = match recv_ty {
            T::DictOf(k, v) => ((**k).clone(), (**v).clone()),
            // 要素型を持たない素の `dict`。容器の形だけは判るので `Any` を入れる。
            T::Dict => (T::Any, T::Any),
            _ => return None,
        };
        // 既定値つきの `get` / `pop` / `setdefault`（タスク 3-6）: 既定値が値の型と同じならその型、`None` なら `Option[V]`、
        // 違えば両方の `Union`。
        let or_default = |d: &T| -> T {
            match d {
                _ if *d == v || matches!(d, T::Never) || matches!(v, T::Any) => v.clone(),
                T::None => T::Union(vec![v.clone(), T::None]),
                other => T::Union(vec![v.clone(), other.clone()]),
            }
        };
        let positional: Vec<&T> = args.iter().filter(|(k, _)| k.is_none()).map(|(_, t)| t).collect();
        match method {
            "key" | "keys" => Some(T::ListOf(Box::new(k))),
            "item" | "values" => Some(T::ListOf(Box::new(v))),
            "items" => Some(T::ListOf(Box::new(T::Tuple(vec![k, v])))),
            // 無ければ `None` を返す（`d.get(k)` は `Option[V]`）。
            "get" | "setdefault" => match positional.as_slice() {
                [_] => Some(or_default(&T::None)),
                [_, d] => Some(or_default(d)),
                _ => None,
            },
            // 無ければ `KeyError`（既定値が無いとき）。
            "pop" => match positional.as_slice() {
                [_] => Some(v),
                [_, d] => Some(or_default(d)),
                _ => None,
            },
            "popitem" => Some(T::Tuple(vec![k, v])),
            "copy" => Some(recv_ty.clone()),
            "update" | "clear" => Some(T::None),
            _ => None,
        }
    }

    pub(super) fn check_builtin_collection_method_args(
        &mut self,
        recv_ty: &InferredType,
        method: &str,
        arg_data: &[(Option<String>, InferredType)],
        span: &crate::token::Span,
    ) {
        use InferredType as T;
        if let T::Ptr(elem) = recv_ty {
            self.check_ptr_method(recv_ty, elem, method, arg_data, span);
            return;
        }
        let elem = match (recv_ty, method) {
            (T::ListOf(e), "append")
            | (T::FixedListOf(e), "append")
            | (T::ListLikeOf(e), "append") => e,
            (T::SetOf(e), "add") | (T::SetOf(e), "discard") | (T::SetOf(e), "remove") => e,
            // `py_list` / `py_set`（D-12・2-7）。⚠ `insert(i, x)` は引数が 2 つなので下の 1 引数の照合に乗らない
            (T::PyList(e), "append" | "remove" | "index" | "count") | (T::PySet(e), "add" | "discard" | "remove") => e,
            // 要素型の判らない素の `list` / `set`、クラス、その他のメソッドは対象外。
            _ => return,
        };
        // 判定材料が無い要素型（`list[Any]`・型変数）は照合しない。
        if matches!(**elem, T::Unresolved | T::Any | T::Never) || self.mentions_type_param(elem) {
            return;
        }
        // ⚠ 引数が 1 つの位置引数でなければ、実装側が個数エラーを出す形。ここでは触らない。
        let [(None, got)] = arg_data else { return };
        // ⚠ protocol 期待型は適合検査へ回す（`check_expected` の doc）。
        let ctx = format!("argument of `{method}`");
        if self.check_expected(got, elem, false, &ctx) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::SubscriptTypeMismatch {
                context: format!("argument of `{recv_ty}.{method}()`"),
                expected: (**elem).clone(),
                got: got.clone(),
            },
            span: Some(span.clone()),
        });
    }

    /// 組み込み関数の呼び出しの引数を、宣言（`src/built_in_stab/builtins.ars`）の仮引数で検査する。
    ///
    /// ⚠⚠ 以前は `range` / `len`（大域の関数型・`check_fn_type_call`）以外の組み込みの引数を
    ///   検査していなかった。`open("a.txt", 1)` / `close("x")` / `getenv(1)` は実行時まで通っていた。
    /// - 仮引数が固定の関数は、関数値の呼び出しと同じ検査（[`Self::check_fn_type_call`]・個数・
    ///   キーワード引数の名前・型）。
    /// - `let ...: T`（`print` / `zip` / `min`）は位置引数をそれぞれ `T` と突き合わせ、`... =` を弾く
    ///   （実行時も通らない）。キーワード引数は、宣言で `let ...` と並べた**固定の仮引数**（既定値つき・
    ///   キーワード専用・`min(xs, key = f)` / `print(x, end = "")`）の名前と型で検査し、ほかは弾く。
    ///   ⚠ 固定の仮引数を `let ...` の**前**に書くのは、Arrow の構文が `...` を最後の仮引数に限るため
    ///   （CPython の `min(*args, key=None)` の並びは書けない）。位置引数は固定の仮引数へ入らない
    ///   （`builtins.ars` の規則・単体テスト `variadic_builtins_fixed_params_are_keyword_only` が守る）。
    /// ⚠ 型の変換（`int(x)` …）は検査しない（`builtins::call_decl`）。
    /// ⚠ 呼ぶのは同じ名前の利用者の関数・変数が無いときだけ（呼ぶ側 `infer_call_inner`）。
    pub(super) fn check_builtin_call(
        &mut self,
        name: &str,
        args: &[CallArg],
        arg_data: &[(Option<String>, InferredType)],
    ) {
        let Some(decl) = crate::type_check::builtins::call_decl(name) else { return };
        let Some(elem) = decl.variadic else {
            self.check_fn_type_call(name, args, arg_data, &decl.params);
            return;
        };
        // ⚠ 展開（`print(*xs)`）は位置で対応づけられない。`check_fn_type_call` と同じく降りる。
        if Self::has_spread_arg(args) {
            return;
        }
        for (i, (key, arg_ty)) in arg_data.iter().enumerate() {
            let at = args.get(i).map(CallArg::expr);
            if let Some(arg_name) = key {
                match decl.params.iter().position(|p| &p.name == arg_name) {
                    Some(param_pos) => {
                        let param = &decl.params[param_pos];
                        if param.ty != InferredType::Any && !self.types_compatible(arg_ty, &param.ty, Site::LetParam) {
                            self.report_error_at_opt(at, StaticTypeError {
                                kind: TypeErrorKind::CallArgTypeMismatch {
                                    func_name: name.to_string(),
                                    param_index: param_pos,
                                    expected: param.ty.clone(),
                                    got: arg_ty.clone(),
                                },
                                span: None,
                            });
                        }
                    }
                    None => self.report_error_at_opt(at, StaticTypeError {
                        kind: TypeErrorKind::UnknownKeywordArg {
                            func_name: name.to_string(),
                            arg_name: arg_name.clone(),
                        },
                        span: None,
                    }),
                }
                continue;
            }
            if elem != InferredType::Any && !self.types_compatible(arg_ty, &elem, Site::LetParam) {
                self.report_error_at_opt(at, StaticTypeError {
                    kind: TypeErrorKind::CallArgTypeMismatch {
                        func_name: name.to_string(),
                        param_index: i,
                        expected: elem.clone(),
                        got: arg_ty.clone(),
                    },
                    span: None,
                });
            }
        }
    }

    /// `Ptr[T]` のメソッド（ffi_value_exchange_plan.md 3-3）。`release()` / `to_list()` / `to_bytes()` だけ。
    ///
    /// - `to_list()` は要素型が分かるときだけ（`Ptr[void]` は `mustbe Ptr[T]` で変換してから）
    /// - `to_bytes()` は文字型の要素（`int8` / `uint8` / `uint16`）のときだけ
    /// - ほかのメソッドは無い（アドレスを取り出す手段を作らない・D-3）
    fn check_ptr_method(
        &mut self,
        recv_ty: &InferredType,
        elem: &crate::type_check::types::PtrElem,
        method: &str,
        arg_data: &[(Option<String>, InferredType)],
        span: &crate::token::Span,
    ) {
        use crate::type_check::types::PtrElem;
        let reason = match method {
            "release" | "to_list" | "to_bytes" if !arg_data.is_empty() => {
                Some(format!("{method}() takes no arguments"))
            }
            "release" => None,
            "to_list" => match elem {
                PtrElem::Void | PtrElem::Unknown => Some(
                    "its element type is unknown, so there is nothing to read; convert it first with `p mustbe Ptr[T]`"
                        .to_string(),
                ),
                PtrElem::Num(_) | PtrElem::Type(_) => None,
            },
            "to_bytes" if elem.is_char() => None,
            "to_bytes" => Some(
                "only a Ptr to character data (int8 / uint8 / uint16) can be copied into bytes; use to_list()"
                    .to_string(),
            ),
            _ => Some("a Ptr only has release(), to_list() and to_bytes() (its address is never exposed)".to_string()),
        };
        if let Some(reason) = reason {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::PtrMethodUnavailable {
                    ptr_ty: recv_ty.clone(),
                    method: method.to_string(),
                    reason,
                },
                span: Some(span.clone()),
            });
        }
    }
}

// eval/builtins.rs — 組み込み関数呼び出しの評価: eval_builtin_ident_call / eval_builtin_open。


use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::interpreter::{
        ByteModeRust, FileData, FileOpenModeRust,
        Interpreter, Value,
    },
};
use super::*;

/// **値として使える**組み込みの名前（python_builtins_plan.md のタスク 1-7）。
///
/// 束縛されていない名前がここにあれば、名前の探索の**最後の段**で `Value::Type(名前)` になる
/// （[`Interpreter::builtin_value`]・CPython の `builtins` と同じ位置）。呼ぶと `call_type_constructor_evaled` が
/// 組み込み関数の表（`eval_builtin_evaled` / キーワードつきは `eval_builtin_evaled_named`）へ回す。
/// ⚠⚠ **大域へ束縛しないこと。** `int` / `str` / `len` は大域に束縛してあるので、同名の最上位の宣言が
///   `already declared` になる（`let dict = 3` が実行時の `NameError`・実測）。ここに足す名前はそうならない。
/// ⚠ `list` / `tuple` は型の名前（呼ぶと作る）。ほかは関数（表示は `<built-in function 名前>`）。
pub(crate) const BUILTIN_VALUE_NAMES: &[&str] = &[
    "print", "range", "next", "repr", "enumerate", "zip", "getenv", "open", "close", "parse_ar",
    "create_flat_int_list", "flat_get_int", "flat_set_int", "list", "tuple",
    // Python の組み込み（python_builtins_plan.md のフェーズ 2〜）
    "isinstance", "type", "getattr", "hasattr", "setattr", "issubclass", "callable",
    "all", "any", "min", "max", "sum", "sorted", "reversed", "abs", "round", "divmod", "pow",
    "map", "filter", "iter", "hash", "ord", "chr", "hex", "oct", "bin", "ascii",
    // 書式指定の小言語（タスク 3-4）
    "format",
    // 変換器が Python の `with` の終わりに置く（ファイルを閉じる・タスク 5-4）
    "__py_with_exit",
    // 変換器が Python の `object()` / 基底が `object` のクラスの `super().__init__(..)` にする（タスク 6-10）
    "__py_object", "__py_object_init",
    // 変換器が Python の `del o.a` / `del d[k]` にする（known_bugs.md の #2 / #3）
    "__py_delattr", "__py_delitem",
    // 変換器が Python の `object.__setattr__(..)` / `object.__delattr__(..)` / `object.__getattribute__(..)` にする
    // （known_bugs.md の #10 / #11）
    "__py_object_setattr", "__py_object_delattr", "__py_object_getattribute",
    // 型（呼ぶと作る・タスク 3-1）
    "frozenset", "bytes",
];

/// `next` の引数の数の誤り（CPython と同じ文言）。
fn next_arity_error(n: usize) -> String {
    if n == 0 {
        "TypeError: next expected at least 1 argument, got 0".to_string()
    } else {
        format!("TypeError: next expected at most 2 arguments, got {n}")
    }
}

/// `open` の失敗を例外の文字列にする。OS のエラーの種類で `OSError` の派生へ振り分ける
/// （CPython と同じ・python_builtins_plan.md のタスク 4-1）。以前はどれも `IOError` だった。
/// ⚠ 文言（`cannot open '..': <OS の文言>`）は変えていない。CPython は `[Errno 2] No such file or directory: '..'`。
fn open_error(e: &std::io::Error, what: &str, file_path: &str) -> String {
    use std::io::ErrorKind as K;
    let class = match e.kind() {
        K::NotFound => "FileNotFoundError",
        K::PermissionDenied => "PermissionError",
        K::AlreadyExists => "FileExistsError",
        K::IsADirectory => "IsADirectoryError",
        K::NotADirectory => "NotADirectoryError",
        _ => "IOError",
    };
    format!("{class}: cannot {what} '{file_path}': {e}")
}

/// 値として取り出したとき**関数**として表示する組み込みの名前か（`<built-in function len>`）。
/// `list` / `tuple` / `type` / `frozenset` / `bytes` と大域の型（`int` …）は型（`<class 'int'>`）。`len` / `id` は大域に束縛された関数。
pub(crate) fn is_builtin_function_name(name: &str) -> bool {
    matches!(name, "len" | "id")
        || (BUILTIN_VALUE_NAMES.contains(&name) && !matches!(name, "list" | "tuple" | "type" | "frozenset" | "bytes"))
}

impl Interpreter {
    /// `enumerate(iterable, start=0)` の引数（3 つの入口で同じ解釈・python_builtins_plan.md のタスク 3-5）。
    /// `start` は位置でもキーワードでも渡せる（CPython と同じ）。文言も CPython と同じ。
    /// ⚠ 以前は 3 つの入口が別々に書かれ、どれも位置の `start`（`enumerate(xs, 1)`）を受けなかった。
    fn enumerate_args(&self, mut positional: Vec<Value>, start_kw: Option<Value>) -> Result<(Value, i64), String> {
        let given = positional.len() + usize::from(start_kw.is_some());
        if positional.is_empty() {
            return Err("TypeError: enumerate() missing required argument 'iterable' (pos 1)".to_string());
        }
        if given > 2 {
            return Err(format!("TypeError: enumerate() takes at most 2 arguments ({given} given)"));
        }
        let start = match start_kw.or_else(|| (positional.len() == 2).then(|| positional.pop().expect("len 2"))) {
            None => 0,
            Some(Value::Int(n)) => n,
            Some(Value::Bool(b)) => i64::from(b),
            Some(other) => {
                return Err(format!(
                    "TypeError: '{}' object cannot be interpreted as an integer",
                    self.type_name(&other)
                ))
            }
        };
        Ok((positional.into_iter().next().expect("len >= 1"), start))
    }

    /// 束縛されていない名前 `name` の**組み込みの値**（名前の探索の最後の段・タスク 1-7）。組み込みでなければ `None`。
    ///
    /// 呼び出し元は名前の読みが失敗した所すべて（ツリーウォークの `eval` の識別子 3 か所・VM の `LoadGlobal` 2 か所）。
    /// ⚠ 束縛した名前が勝つ（ここへ来るのは束縛が無いときだけ）ので、`let repr = f` の利用者の値を隠さない。
    /// ⚠ `property` / `cached_property` は Arrow で書いたクラス（タスク 6-1 / 6-5）。初めて引いたときに組み立てるので `&mut self`。
    pub(crate) fn builtin_value(&mut self, name: &str) -> Option<Value> {
        // 組み込みのクラス `object`（タスク 6-10）。⚠ 呼んでも作れない（作れるのは Python の `object()` だけ）。
        if name == "object" {
            return Some(Value::Class(self.object_class()));
        }
        // 唯一の値 `Ellipsis`（Python の `...` は変換器がこの名前にする・タスク 6-11）。
        if name == "Ellipsis" {
            return Some(self.builtin_singleton(crate::interpreter::BuiltinClass::Ellipsis));
        }
        // 唯一の値 `NotImplemented`（演算子メソッドが「答えない」と返す・タスク 6-12）。
        if name == "NotImplemented" {
            return Some(self.builtin_singleton(crate::interpreter::BuiltinClass::NotImplemented));
        }
        if super::py_builtins::PY_BUILTIN_CLASS_NAMES.contains(&name) {
            return match self.py_builtin_class(name) {
                Ok(class) => Some(class),
                Err(e) => {
                    eprintln!("{e}");
                    None
                }
            };
        }
        BUILTIN_VALUE_NAMES.contains(&name).then(|| Value::Type(name.to_string()))
    }

    /// 組み込み関数名を受け取り、該当する組み込みを実行して結果を返す。
    /// 未知の名前には `None` を返してユーザー定義関数の探索にフォールスルーする。
    /// 評価済み引数で「純粋・共通」な組み込みを呼ぶ（VM の `CallBuiltin` op 用）。
    /// `eval_builtin_ident_call` の対応アームと**同一意味論**（引数は VM がスタックで評価済み）。
    /// ここで扱わない名前は `None`（コンパイラは扱う名前だけ `CallBuiltin` を発行する）。
    /// `print` の 1 行を出す（設計書 §4.1 / タスク 3-4）。
    ///
    /// ⚠⚠ **展開中は stderr へ。** stdout に出すと `compare_python_impl` /
    /// `compare_outputs` の差分比較が壊れる ——「意味論を守る唯一の網」が黙って無効になる。
    /// ⚠ `print` の出口を**ここ 1 箇所**にまとめてある（評価済み引数版と AST 版の 2 つが
    /// あるので、片方だけ直すと展開時の出力が漏れる）。
    pub(crate) fn emit_print(&self, line: &str) {
        if self.meta_expanding {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }

    /// 改行を足さずに書く（`print(.., end=..)`・タスク 5-1）。行き先は `emit_print` と同じ。
    pub(crate) fn emit_text(&self, text: &str, flush: bool) {
        use std::io::Write;
        if self.meta_expanding {
            eprint!("{text}");
        } else {
            print!("{text}");
            if flush {
                let _ = std::io::stdout().flush();
            }
        }
    }

    pub(crate) fn eval_builtin_evaled(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Option<Result<Value, String>> {
        // ⚠ 展開時の禁止ビルトイン（D26・タスク 3-7）。3 つの入口すべてに置く。
        if let Some(e) = self.meta_forbidden_builtin(name) {
            return Some(Err(e));
        }
        match name {
            "print" => {
                let mut parts: Vec<String> = Vec::with_capacity(args.len());
                for v in &args {
                    match self.display_str(v) {
                        Ok(s) => parts.push(s),
                        Err(e) => return Some(Err(e)),
                    }
                }
                self.emit_print(&parts.join(" "));
                Some(Ok(Value::None))
            }
            // `parse_ar(source[, path])`（#56）。**入力は評価済みの文字列だけ**なので
            // 評価済み引数で完全に表現できる（AST が要るのは**出力**＝`Value::Namespace` ツリーの側）。
            // ⚠ #33 でフォールバックが消えた後も、#56 で削除した `is_builtin_callee` が bail し続け、
            //    **`parse_ar` は `VmForceError` で完全に死んでいた**（#55 で検出）。
            "parse_ar" => Some(self.parse_ar_evaled(args)),
            // `list(it)` / `tuple(it)`（タスク 1-7）/ `frozenset(it)`（3-1）。本体は型の呼び出し（`call_type_by_name_evaled`）に 1 本化。
            "list" | "tuple" | "type" | "frozenset" | "bytes" => Some(self.call_type_by_name_evaled(name, args)),
            // ── Python の組み込み（python_builtins_plan.md のフェーズ 2・本体は `eval/py_builtins.rs`）──
            "isinstance" => Some(match args.as_slice() {
                [x, spec] => self.py_isinstance(x, spec).map(Value::Bool),
                _ => Err(format!("TypeError: isinstance expected 2 arguments, got {}", args.len())),
            }),
            "issubclass" => Some(match args.as_slice() {
                [c, spec] => self.py_issubclass(c, spec).map(Value::Bool),
                _ => Err(format!("TypeError: issubclass expected 2 arguments, got {}", args.len())),
            }),
            "callable" => Some(match args.as_slice() {
                [x] => Ok(Value::Bool(self.py_callable(x))),
                _ => Err(format!("TypeError: callable() takes exactly one argument ({} given)", args.len())),
            }),
            // 集計（タスク 3-2）。キーワード引数つきは `eval_builtin_evaled_named`。
            "all" => Some(self.py_all_any(args, true)),
            "any" => Some(self.py_all_any(args, false)),
            "min" => Some(self.py_min_max(args, Vec::new(), false)),
            "max" => Some(self.py_min_max(args, Vec::new(), true)),
            "sum" => Some(self.py_sum(args, Vec::new())),
            "sorted" => Some(self.py_sorted(args, Vec::new())),
            "reversed" => Some(self.py_reversed(args)),
            "abs" => Some(self.py_abs(args)),
            "round" => Some(self.py_round(args, Vec::new())),
            "divmod" => Some(self.py_divmod(args)),
            "pow" => Some(self.py_pow(args, Vec::new())),
            // 変換・遅延の組み込み（タスク 3-3）。
            "map" => Some(self.py_map(args)),
            "filter" => Some(self.py_filter(args)),
            "iter" => Some(self.py_iter(args)),
            "hash" => Some(self.py_hash(args)),
            "ord" => Some(self.py_ord_chr(args, true)),
            "chr" => Some(self.py_ord_chr(args, false)),
            "hex" => Some(self.py_int_to_base(args, 16)),
            "oct" => Some(self.py_int_to_base(args, 8)),
            "bin" => Some(self.py_int_to_base(args, 2)),
            "ascii" => Some(self.py_ascii(args)),
            // 書式指定の小言語（タスク 3-4）。`str.format` / f-string の `{x:spec}` と同じ本体（`format_value`）。
            "format" => Some(self.py_format(args)),
            // Python の `with` の終わり（変換器が `finally` に置く・タスク 5-4）。ファイルなら閉じる（CPython の `__exit__`）、
            // ほかの値は何もしない（従来どおりブロックの破棄に任せる）。
            "__py_with_exit" => Some(match args.as_slice() {
                [Value::FileObject(fd)] => {
                    fd.borrow_mut().close();
                    Ok(Value::None)
                }
                [_] => Ok(Value::None),
                _ => Err(format!("TypeError: __py_with_exit() takes exactly one argument ({} given)", args.len())),
            }),
            // Python の `object()` / 基底が `object` のクラスの `super().__init__(..)`（変換器が置く・タスク 6-10）。
            "__py_object" => Some(self.py_object(&args)),
            "__py_object_init" => Some(self.py_object_init(&args)),
            // Python の `del o.a` / `del d[k]`（変換器が置く・known_bugs.md の #2 / #3）。
            "__py_delattr" => Some(match <[Value; 2]>::try_from(args) {
                Ok([obj, Value::Str(name)]) => self.del_attr_val(obj, &name, true).map(|_| Value::None),
                Ok(_) => Err("TypeError: __py_delattr() takes an object and an attribute name".to_string()),
                Err(a) => Err(format!("TypeError: __py_delattr() takes exactly 2 arguments ({} given)", a.len())),
            }),
            // Python の `object.__setattr__(o, k, v)` / `object.__delattr__(o, k)` / `object.__getattribute__(o, k)`
            // （上書きを飛ばす・変換器が置く・#10 / #11）。
            "__py_object_setattr" => Some(self.py_object_setattr(args)),
            "__py_object_delattr" => Some(self.py_object_delattr(args)),
            "__py_object_getattribute" => Some(self.py_object_getattribute(args)),
            "__py_delitem" => Some(match <[Value; 2]>::try_from(args) {
                Ok([obj, key]) => self.eval_delitem(obj, key).map(|_| Value::None),
                Err(a) => Err(format!("TypeError: __py_delitem() takes exactly 2 arguments ({} given)", a.len())),
            }),
            "getattr" => Some(self.py_getattr(args)),
            "hasattr" => Some(self.py_hasattr(args)),
            "setattr" => Some(self.py_setattr(args)),
            // flat リスト組み込み（#27-c）。ツリーウォーク側と**同一の本体**へ委譲する。
            "create_flat_int_list" | "flat_get_int" | "flat_set_int" => {
                Some(self.eval_builtin_flat_evaled(name, args))
            }
            // ファイル操作（#27-c）。VM は位置引数だけを積むので、名前無しの 3 つ組に直して
            // ツリーウォークと同じ本体へ渡す（キーワード引数つきの呼び出しはコンパイラが bail する）。
            "open" => Some(
                self.eval_builtin_open_evaled(args.into_iter().map(|v| (None, v, true)).collect()),
            ),
            "close" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: close() takes exactly one argument".to_string()
                    ));
                }
                let val = args.into_iter().next().unwrap();
                Some(match val {
                    Value::FileObject(fd_rc) => {
                        // タスクへ送った複製（7-19）
                        if fd_rc.borrow().detached {
                            return Some(crate::interpreter::async_mgr::ensure_attached(&Value::FileObject(fd_rc)).map(|_| Value::None));
                        }
                        fd_rc.borrow_mut().close();
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: close() argument must be FileObject, not '{}'",
                        self.type_name(&other)
                    )),
                })
            }
            "range" => Some(match args.as_slice() {
                [Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                    (0..*stop).map(Value::Int).collect(),
                )))),
                [Value::Int(start), Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                    (*start..*stop).map(Value::Int).collect(),
                )))),
                [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                    let mut items = Vec::new();
                    let mut i = *start;
                    if *step > 0 {
                        while i < *stop {
                            items.push(Value::Int(i));
                            i += step;
                        }
                    } else if *step < 0 {
                        while i > *stop {
                            items.push(Value::Int(i));
                            i += step;
                        }
                    }
                    Ok(Value::List(Rc::new(RefCell::new(items))))
                }
                _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
            }),
            "len" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: len() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(self.len_of(val))
            }
            // `next(it)` / `next(it, default)`（本体は `py_next`・タスク 3-5）。
            "next" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(next_arity_error(args.len())));
                }
                let mut it = args.into_iter();
                let v = it.next().expect("len >= 1");
                Some(self.py_next(v, it.next()))
            }
            "repr" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: repr() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(self.repr_val(&val).map(Value::str))
            }
            "id" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: id() takes exactly one argument".to_string()));
                }
                let val = args.into_iter().next().unwrap();
                Some(self.call_type_by_name_evaled("id", vec![val]))
            }
            // VM は位置引数のみ渡す（`start=` キーワードは `eval_builtin_evaled_named`）。`enumerate(it, 10)` も受ける。
            "enumerate" => Some(
                self.enumerate_args(args, None).and_then(|(it, start)| self.py_enumerate(it, start)),
            ),
            "zip" => Some(self.py_zip(args)),
            "getenv" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: getenv() takes 1 or 2 arguments (name[, default])".to_string(),
                    ));
                }
                let mut it = args.into_iter();
                let name = match it.next().unwrap() {
                    Value::Str(s) => s,
                    other => {
                        return Some(Err(format!(
                            "TypeError: getenv() name must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                };
                let default = match it.next() {
                    Some(Value::Str(s)) => s,
                    Some(other) => {
                        return Some(Err(format!(
                            "TypeError: getenv() default must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    None => Rc::from(""),
                };
                Some(Ok(Value::str(
                    std::env::var(&*name).unwrap_or_else(|_| default.to_string()),
                )))
            }
            _ => None,
        }
    }

    pub(crate) fn eval_builtin_ident_call(
        &mut self,
        name: &str,
        args: &[CallArg],
    ) -> Option<Result<Value, String>> {
        // #55: AST 式を取るツリーウォーク入口の通過を数える（既定ビルドでは消える）。
        crate::interpreter::tw_stats::record_site(4);
        // ⚠ 展開時の禁止ビルトイン（D26・タスク 3-7）。
        if let Some(e) = self.meta_forbidden_builtin(name) {
            return Some(Err(e));
        }
        match name {
            "print" => {
                // キーワード引数（`sep=` / `end=`・タスク 5-1）があれば評価済みの入口へ回す。
                // ⚠ 腕のガード（`"print" if ..`）にしないこと。`checker_knows_every_runtime_builtin_name` が
                //   `"名前" =>` の行を名前の表として読む。
                if args.iter().any(|a| matches!(a, CallArg::Keyword { .. })) {
                    let mut named: Vec<(Option<String>, Value)> = Vec::with_capacity(args.len());
                    for a in args {
                        let (key, e) = match a {
                            CallArg::Keyword { name, value } => (Some(name.clone()), value),
                            other => (None, other.expr()),
                        };
                        match self.eval(e) {
                            Ok(v) => named.push((key, v)),
                            Err(e) => return Some(Err(e)),
                        }
                    }
                    return Some(self.py_print(named));
                }
                let mut parts: Vec<String> = Vec::new();
                for a in args {
                    let v = match self.eval(a.expr()) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    let s = match self.display_str(&v) {
                        Ok(s) => s,
                        Err(e) => return Some(Err(e)),
                    };
                    parts.push(s);
                }
                self.emit_print(&parts.join(" "));
                Some(Ok(Value::None))
            }
            // `next(it)` / `next(it, default)`（評価済みの入口と同じ本体 `py_next`・タスク 3-5）。
            "next" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(next_arity_error(args.len())));
                }
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    match self.eval(a.expr()) {
                        Ok(v) => vals.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                let mut it = vals.into_iter();
                let v = it.next().expect("len >= 1");
                Some(self.py_next(v, it.next()))
            }
            "repr" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: repr() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.repr_val(&val).map(Value::str))
            }
            "range" => {
                let evaled: Result<Vec<_>, _> = args.iter().map(|a| self.eval(a.expr())).collect();
                let evaled = match evaled {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match evaled.as_slice() {
                    [Value::Int(stop)] => Ok(Value::List(Rc::new(RefCell::new(
                        (0..*stop).map(Value::Int).collect(),
                    )))),
                    [Value::Int(start), Value::Int(stop)] => Ok(Value::List(Rc::new(
                        RefCell::new((*start..*stop).map(Value::Int).collect()),
                    ))),
                    [Value::Int(start), Value::Int(stop), Value::Int(step)] => {
                        let mut items = Vec::new();
                        let mut i = *start;
                        if *step > 0 {
                            while i < *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        } else if *step < 0 {
                            while i > *stop {
                                items.push(Value::Int(i));
                                i += step;
                            }
                        }
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    _ => Err("TypeError: range() takes 1\u{2013}3 integer arguments".to_string()),
                })
            }
            "len" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: len() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.len_of(val))
            }
            // ── mutable flat-list built-ins ───────────────────────────────────
            // 本体は `eval_builtin_flat_evaled`（評価済み引数版）に 1 本化してある（#27-c）。
            // ここは引数を評価して委譲するだけ（`*_evaled` とずれた実装を作らない — #22 系列）。
            "create_flat_int_list" | "flat_get_int" | "flat_set_int" => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    match self.eval(a.expr()) {
                        Ok(v) => vals.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Some(self.eval_builtin_flat_evaled(name, vals))
            }
            // ─────────────────────────────────────────────────────────────────
            "id" => {
                if args.len() != 1 {
                    return Some(Err("TypeError: id() takes exactly one argument".to_string()));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(self.call_type_by_name_evaled("id", vec![val]))
            }
            "open" => Some(self.eval_builtin_open(args)),
            "close" => {
                if args.len() != 1 {
                    return Some(Err(
                        "TypeError: close() takes exactly one argument".to_string()
                    ));
                }
                let val = match self.eval(args[0].expr()) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                };
                Some(match val {
                    Value::FileObject(fd_rc) => {
                        // タスクへ送った複製（7-19）
                        if fd_rc.borrow().detached {
                            return Some(crate::interpreter::async_mgr::ensure_attached(&Value::FileObject(fd_rc)).map(|_| Value::None));
                        }
                        fd_rc.borrow_mut().close();
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: close() argument must be FileObject, not '{}'",
                        self.type_name(&other)
                    )),
                })
            }
            "enumerate" => {
                let mut positional: Vec<Value> = Vec::new();
                let mut start_val: Option<Value> = None;
                for arg in args {
                    match arg {
                        CallArg::Positional(e) => match self.eval(e) {
                            Ok(v) => positional.push(v),
                            Err(e) => return Some(Err(e)),
                        },
                        CallArg::Keyword { name, value } if name == "start" => {
                            match self.eval(value) {
                                Ok(v) => start_val = Some(v),
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        CallArg::Keyword { name, .. } => {
                            return Some(Err(format!(
                                "TypeError: enumerate() got unexpected keyword argument '{name}'"
                            )));
                        }
                        CallArg::Variadic(_) => {
                            return Some(Err(
                                "TypeError: enumerate() does not support variadic arguments".to_string()
                            ));
                        }
                        // ⚠ `enumerate` は引数を自前で読むので展開を扱えない。
                        //   黙って捨てず明示エラーにする（`enumerate(*xs)` は稀）。
                        CallArg::Spread(_) | CallArg::KwSpread(_) => {
                            return Some(Err(
                                "TypeError: enumerate() does not support `*`/`**` argument unpacking"
                                    .to_string(),
                            ));
                        }
                    }
                }
                Some(self.enumerate_args(positional, start_val).and_then(|(it, start)| self.py_enumerate(it, start)))
            }
            "zip" => {
                for arg in args.iter() {
                    if matches!(arg, CallArg::Keyword { .. }) {
                        return Some(Err(
                            "TypeError: zip() takes no keyword arguments".to_string()
                        ));
                    }
                }
                let mut iters_vals: Vec<Value> = Vec::new();
                for arg in args.iter() {
                    match self.eval(arg.expr()) {
                        Ok(v) => iters_vals.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                Some(self.py_zip(iters_vals))
            }
            "getenv" => {
                if args.is_empty() || args.len() > 2 {
                    return Some(Err(
                        "TypeError: getenv() takes 1 or 2 arguments (name[, default])".to_string(),
                    ));
                }
                let name = match self.eval(args[0].expr()) {
                    Ok(Value::Str(s)) => s,
                    Ok(other) => {
                        return Some(Err(format!(
                            "TypeError: getenv() name must be str, not '{}'",
                            self.type_name(&other)
                        )))
                    }
                    Err(e) => return Some(Err(e)),
                };
                let default = if args.len() > 1 {
                    match self.eval(args[1].expr()) {
                        Ok(Value::Str(s)) => s,
                        Ok(other) => {
                            return Some(Err(format!(
                                "TypeError: getenv() default must be str, not '{}'",
                                self.type_name(&other)
                            )))
                        }
                        Err(e) => return Some(Err(e)),
                    }
                } else {
                    Rc::from("")
                };
                Some(Ok(Value::str(
                    std::env::var(&*name).unwrap_or_else(|_| default.to_string()),
                )))
            }
            // `parse_ar`（#56）。**唯一の実装は `eval_builtin_evaled` 側**。ここは引数を評価して
            // 委譲するだけにする（`*_evaled` 版とずれた実装を作らない — 実バグ 4 回の教訓）。
            "parse_ar" => {
                let mut vals: Vec<Value> = Vec::with_capacity(args.len());
                for a in args {
                    match self.eval(a.expr()) {
                        Ok(v) => vals.push(v),
                        Err(e) => return Some(Err(e)),
                    }
                }
                self.eval_builtin_evaled("parse_ar", vals)
            }
            _ => None,
        }
    }

    /// `parse_ar(source[, path])` の**唯一の実装**（#56）。
    ///
    /// ソースを字句解析・構文解析して、AST を `Value::Namespace` ツリーへ変換して返す（§2.1）。
    /// `python_converter` / `converter.ar` がこれに依存する。
    ///
    /// ⚠ **入力は文字列だけ**なので評価済み引数で表現できる。#56 で削除した `is_builtin_callee` の
    /// 「AST を値へ変換するので評価済み引数では表現できない」は**出力と入力を取り違えていた**。
    /// その誤りのせいで #33 以降 `parse_ar` は `VmForceError` で停止していた（#55 で検出・#56 で修正）。
    pub(crate) fn parse_ar_evaled(&mut self, args: Vec<Value>) -> Result<Value, String> {
        if args.is_empty() || args.len() > 2 {
            return Err("TypeError: parse_ar() takes 1 or 2 arguments (source[, path])".to_string());
        }
        let mut it = args.into_iter();
        let source = match it.next().unwrap() {
            Value::Str(s) => s,
            other => {
                return Err(format!(
                    "TypeError: parse_ar() source must be str, not '{}'",
                    self.type_name(&other)
                ))
            }
        };
        let path = match it.next() {
            Some(Value::Str(s)) => s,
            Some(other) => {
                return Err(format!(
                    "TypeError: parse_ar() path must be str, not '{}'",
                    self.type_name(&other)
                ))
            }
            None => Rc::from(""),
        };
        let source: &str = source.strip_prefix('\u{FEFF}').unwrap_or(&source);
        let path: &str = &path;
        let tokens = crate::lexer::Lexer::new(source, path).tokenize();
        let source_dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
        let stmts = match crate::parser::Parser::new(tokens, source_dir).parse_program() {
            Ok(s) => s,
            Err(e) => return Err(format!("ParseError in parse_ar: {e}")),
        };
        Ok(crate::interpreter::ast_value::stmts_to_value(&stmts))
    }

    /// 評価済み引数＋**引数名**の組み込み呼び出し（VM の `Op::CallBuiltinKw` 用・#27-c）。
    ///
    /// `eval_builtin_evaled` は位置引数しか受け取れないので、`enumerate(xs, start=1)` のような
    /// 形はコンパイラが bail していた。ここはキーワードの解釈が**ツリーウォークと一致すると
    /// 確認できた組み込みだけ**を扱う（`vm::compiler::VM_BUILTIN_KW_NAMES` と対応）。
    ///
    /// ⚠ **名前ごとにキーワードの扱いが違う**（`enumerate` は `start` だけ許容、`zip` はエラー、
    /// `len` は名前を無視して位置引数として扱う…）。一致を確認していない名前をここへ足すと
    /// off/auto がずれるので、必ず `eval_builtin_ident_call` の当該アームと突き合わせること。
    pub(crate) fn eval_builtin_evaled_named(
        &mut self,
        name: &str,
        args: Vec<(Option<String>, Value)>,
    ) -> Option<Result<Value, String>> {
        // ⚠ 展開時の禁止ビルトイン（D26・タスク 3-7）。
        if let Some(e) = self.meta_forbidden_builtin(name) {
            return Some(Err(e));
        }
        match name {
            // `print(.., sep=.., end=..)`（タスク 5-1）。`eval_builtin_ident_call` の `print` アームも同じ関数へ回す。
            "print" => Some(self.py_print(args)),
            // `super().__init__(*args, **kwargs)` が `object` に届いた形（タスク 6-10）。展開してから数える
            // （空の `**kwargs` は通す・CPython と同じ）。キーワード引数も「余分な引数」。
            "__py_object_init" => {
                let evaled = args.into_iter().map(|(k, v)| (k, v, true)).collect();
                Some(self.expand_spread_args(evaled).and_then(|all| {
                    let values: Vec<Value> = all.into_iter().map(|(_, v, _)| v).collect();
                    self.py_object_init(&values)
                }))
            }
            // `eval_builtin_ident_call` の `enumerate` アームと同じ引数解釈。
            "enumerate" => {
                let mut positional: Vec<Value> = Vec::new();
                let mut start_val: Option<Value> = None;
                for (key, v) in args {
                    match key.as_deref() {
                        None => positional.push(v),
                        Some("start") => start_val = Some(v),
                        Some(other) => {
                            return Some(Err(format!(
                                "TypeError: enumerate() got unexpected keyword argument '{other}'"
                            )))
                        }
                    }
                }
                Some(self.enumerate_args(positional, start_val).and_then(|(it, start)| self.py_enumerate(it, start)))
            }
            // `open` の評価済み版は元から名前つきの 3 つ組を受け取る（そのまま渡せる）。
            "open" => Some(
                self.eval_builtin_open_evaled(
                    args.into_iter().map(|(k, v)| (k, v, true)).collect(),
                ),
            ),
            // キーワード引数を取る Python の組み込み（タスク 3-2・本体は `eval/py_builtins.rs`）。
            "min" | "max" | "sum" | "sorted" | "round" | "pow" => {
                let (pos, kw) = super::py_builtins::split_named(args);
                Some(match name {
                    "min" => self.py_min_max(pos, kw, false),
                    "max" => self.py_min_max(pos, kw, true),
                    "sum" => self.py_sum(pos, kw),
                    "sorted" => self.py_sorted(pos, kw),
                    "round" => self.py_round(pos, kw),
                    _ => self.py_pow(pos, kw),
                })
            }
            // ここに無い名前はコンパイラが `CallBuiltinKw` を発行しない（到達しない）。
            _ => None,
        }
    }

    /// flat リスト組み込み（`create_flat_int_list` / `flat_get_int` / `flat_set_int`）の**唯一の実装**。
    ///
    /// ツリーウォーク（`eval_builtin_ident_call`）と VM（`CallBuiltin` → `eval_builtin_evaled`）の
    /// 両方がここへ委譲する（#27-c）。以前はツリーウォーク側にしか無く、VM は `is_builtin_callee` で
    /// bail していた（`langtons_ant.ar` / `langtons_ant_profile.ar` の最上位が丸ごとツリーウォーク）。
    ///
    /// 全て**評価済み引数の純関数**（副作用は `flat_set_int` のバッファ書き込みのみ）なので、
    /// 評価順の違いは生じない。
    pub(crate) fn eval_builtin_flat_evaled(
        &mut self,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        match name {
            // create_flat_int_list(size, val) → fixed_list[Cell]
            // フラットなバイト列を直接確保する（Cell インスタンスは作らない）。
            // 'Cell' クラスがスコープに要る（`from ant_render import Cell`）。
            "create_flat_int_list" => {
                if args.len() != 2 {
                    return Err(
                        "TypeError: create_flat_int_list() takes exactly 2 arguments".to_string()
                    );
                }
                let size = match &args[0] {
                    Value::Int(n) => *n as usize,
                    _ => {
                        return Err(
                            "TypeError: create_flat_int_list: size must be int".to_string()
                        )
                    }
                };
                let init = match &args[1] {
                    Value::Int(n) => *n,
                    _ => {
                        return Err("TypeError: create_flat_int_list: val must be int".to_string())
                    }
                };
                let cell_class = match self.get_val("Cell") {
                    Some(Value::Class(c)) => c,
                    _ => {
                        return Err(
                            "NameError: create_flat_int_list requires 'Cell' class in scope"
                                .to_string(),
                        )
                    }
                };
                let init_bytes = init.to_le_bytes();
                let mut raw = vec![0u8; size * 8];
                for chunk in raw.chunks_exact_mut(8) {
                    chunk.copy_from_slice(&init_bytes);
                }
                let flat_data = crate::interpreter::value::FlatListData {
                    data: raw,
                    len: size,
                    allocated_size: size,
                };
                let layout = crate::interpreter::value::FlatLayout {
                    class_name: "Cell".to_string(),
                    fields: vec![(
                        "v".to_string(),
                        crate::interpreter::value::FlatFieldTy::Int,
                    )],
                    stride: 8,
                    class: cell_class,
                };
                Ok(Value::FrozenList {
                    state: Rc::new(RefCell::new(flat_data)),
                    layout: Rc::new(layout),
                })
            }
            // flat_get_int(grid, idx) → int
            "flat_get_int" => {
                if args.len() != 2 {
                    return Err("TypeError: flat_get_int() takes exactly 2 arguments".to_string());
                }
                let idx = match &args[1] {
                    Value::Int(n) => *n as usize,
                    _ => return Err("TypeError: flat_get_int: idx must be int".to_string()),
                };
                match &args[0] {
                    Value::FrozenList { state, .. } => {
                        let s = state.borrow();
                        if idx >= s.len {
                            return Err(format!(
                                "IndexError: flat_get_int index {idx} out of range (len {})",
                                s.len
                            ));
                        }
                        let off = idx * 8;
                        let bytes: [u8; 8] = s.data[off..off + 8].try_into().unwrap();
                        Ok(Value::Int(i64::from_le_bytes(bytes)))
                    }
                    other => Err(format!(
                        "TypeError: flat_get_int expects fixed_list, got {}",
                        self.type_name(other)
                    )),
                }
            }
            // flat_set_int(grid, idx, val) → None — フラットバッファへ直接書き込む
            "flat_set_int" => {
                if args.len() != 3 {
                    return Err("TypeError: flat_set_int() takes exactly 3 arguments".to_string());
                }
                let idx = match &args[1] {
                    Value::Int(n) => *n as usize,
                    _ => return Err("TypeError: flat_set_int: idx must be int".to_string()),
                };
                let val = match &args[2] {
                    Value::Int(n) => *n,
                    _ => return Err("TypeError: flat_set_int: val must be int".to_string()),
                };
                match &args[0] {
                    Value::FrozenList { state, .. } => {
                        let mut s = state.borrow_mut();
                        if idx >= s.len {
                            return Err(format!(
                                "IndexError: flat_set_int index {idx} out of range"
                            ));
                        }
                        let off = idx * 8;
                        s.data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                        Ok(Value::None)
                    }
                    other => Err(format!(
                        "TypeError: flat_set_int expects fixed_list, got {}",
                        self.type_name(other)
                    )),
                }
            }
            other => Err(format!("NameError: '{other}' is not defined")),
        }
    }

    /// 組み込み関数 `open()` を実行してファイルオブジェクト (`Value::FileObject`) を返す。
    /// 引数 `file_path`, `open_mode`, `start_point`, `byte_recognizing`, `encoding`, `exclusion` を解析し、
    /// 対応する `std::fs::OpenOptions` を構築してファイルを開く。
    pub(crate) fn eval_builtin_open(&mut self, args: &[CallArg]) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        self.eval_builtin_open_evaled(evaled)
    }

    /// 評価済み引数版の `open()`（VM の `CallBuiltin` 用・#27-c）。`eval_builtin_open` の本体。
    pub(crate) fn eval_builtin_open_evaled(
        &mut self,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        use std::collections::HashMap as HMap;
        use std::fs::OpenOptions;
        use std::io::Read as IoRead;
        let mut kw: HMap<String, Value> = HMap::new();
        let mut pos: Vec<Value> = Vec::new();
        for (k, v, _) in evaled {
            match k {
                Some(n) => {
                    kw.insert(n, v);
                }
                None => pos.push(v),
            }
        }
        // Python の形（`open(path, "w", encoding="utf-8")`・タスク 5-2）は Arrow の列挙の値へ写して同じ道を通す。
        let py_form = super::py_builtins::py_open_form(&pos, &kw)?;
        let file_path = extract_path_str(
            get_arg(&pos, &kw, 0, "file_path")
                .or_else(|| py_form.as_ref().and_then(|_| kw.get("file")))
                .ok_or("TypeError: open() missing required argument 'file_path'")?,
        )?;
        let open_mode_int = match &py_form {
            Some(f) => f.open_mode_for(&file_path),
            None => extract_enum_int(
                get_arg(&pos, &kw, 1, "open_mode")
                    .ok_or("TypeError: open() missing required argument 'open_mode'")?,
                "FileOpenMode",
            )?,
        };
        let start_point_int: i64 = match &py_form {
            Some(f) => f.start_point_for(&file_path),
            None => get_arg(&pos, &kw, 2, "start_point")
                .map(|v| extract_enum_int(v, "StartPoint"))
                .transpose()?
                .unwrap_or(0),
        };
        let byte_mode_int: i64 = match &py_form {
            Some(f) => i64::from(!f.binary),
            None => get_arg(&pos, &kw, 3, "byte_recognizing")
                .map(|v| extract_enum_int(v, "ByteRecognizingMode"))
                .transpose()?
                .unwrap_or(1),
        };
        let enc_int: i64 = match &py_form {
            Some(f) => f.enc_int,
            None => get_arg(&pos, &kw, 4, "encoding")
                .map(|v| extract_enum_int(v, "Encoding"))
                .transpose()?
                .unwrap_or(1),
        };
        if enc_int == 3 {
            return Err("NotImplementedError: Shift-JIS encoding is not yet supported".to_string());
        }
        // ⚠ Python の形の 6 番目は `newline`（`py_open_form` が見た）。
        let _exclusion: bool = get_arg(&pos, &kw, 5, "exclusion")
            .filter(|_| py_form.is_none())
            .map(|v| match v {
                Value::Bool(b) => Ok(*b),
                _ => Err("TypeError: open() 'exclusion' must be bool".to_string()),
            })
            .transpose()?
            .unwrap_or(true);

        let mode = match open_mode_int {
            0 => FileOpenModeRust::Write,
            1 => FileOpenModeRust::Rewrite,
            2 => FileOpenModeRust::Read,
            3 => FileOpenModeRust::MakeAndWrite,
            n => return Err(format!("TypeError: invalid FileOpenMode value {n}")),
        };
        let byte_mode = if byte_mode_int == 0 {
            ByteModeRust::Byte
        } else {
            ByteModeRust::Text
        };

        let std_path = std::path::Path::new(&file_path);
        // ⚠ Python の形の `"x"` は CPython と同じ `FileExistsError`（`create_new` の失敗・`open_error`）にする。
        if mode == FileOpenModeRust::MakeAndWrite && std_path.exists() && py_form.is_none() {
            return Err(format!(
                "RuntimeError: open() make_and_write: file '{}' already exists",
                file_path
            ));
        }

        let (file, content) = match mode {
            FileOpenModeRust::Read => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .open(std_path)
                    .map_err(|e| open_error(&e, "open", &file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| open_error(&e, "read", &file_path))?;
                (f, c)
            }
            FileOpenModeRust::Write => {
                let mut f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(std_path)
                    .map_err(|e| open_error(&e, "open", &file_path))?;
                let mut c = Vec::new();
                f.read_to_end(&mut c)
                    .map_err(|e| open_error(&e, "read", &file_path))?;
                (f, c)
            }
            FileOpenModeRust::Rewrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(std_path)
                    .map_err(|e| open_error(&e, "open", &file_path))?;
                (f, Vec::new())
            }
            FileOpenModeRust::MakeAndWrite => {
                let f = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(std_path)
                    .map_err(|e| open_error(&e, "create", &file_path))?;
                (f, Vec::new())
            }
        };

        let (content, bom_skip) = if enc_int == 2 && content.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (content[3..].to_vec(), 3usize)
        } else {
            (content, 0usize)
        };
        let _ = bom_skip;
        let pointer = if start_point_int == 1 {
            content.len()
        } else {
            0
        };

        let fd = FileData {
            path: file_path,
            mode,
            byte_mode,
            content,
            pointer,
            is_closed: false,
            file_handle: Some(file),
            detached: false,
            // Python の形で開いたファイルは read / write を CPython の意味にする（タスク 5-4）。
            py_mode: py_form.map(|f| f.mode),
        };
        Ok(Value::FileObject(Rc::new(RefCell::new(fd))))
    }

}

impl Interpreter {
    /// 組み込みの `len(x)` の本体（7-18）。`__len__` を持つインスタンスはそれを呼ぶ。
    ///
    /// ⚠ 以前は 3 つの呼び出し経路（`eval/builtins.rs` に 2 つ・`eval/calls.rs` に 1 つ）がそれぞれ同じ規則を写していて、
    ///   3-1 で `bytes` を足したときに `calls.rs` の写しだけ取りこぼした（Python のオブジェクトも無かった）。型注釈の無い
    ///   経路（非同期タスクの本体など）の `len(bytes)` が TypeError になっていた。
    pub(crate) fn len_of(&mut self, val: Value) -> Result<Value, String> {
        let has_instance_len = match &val {
            Value::Instance(inst_rc) => inst_rc.borrow().class.methods.contains_key("__len__"),
            _ => false,
        };
        if has_instance_len {
            // borrow を落としてから呼ぶ
            return self.eval_method_call_evaled(val, "__len__", vec![]).and_then(|r| match r {
                Value::Int(n) => Ok(Value::Int(n)),
                other => Err(format!("TypeError: __len__ must return int, not '{}'", self.type_name(&other))),
            });
        }
        match &val {
            Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
            Value::FrozenList { state, .. } => Ok(Value::Int(state.borrow().len as i64)),
            // 文字数（ffi_value_exchange_plan.md 7-2。添字・反復と同じ単位。⚠ 以前は UTF-8 のバイト数で、`len("日本語")` が 9
            // だった。バイト数は `len(s.encode("utf-8"))`）
            Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
            Value::Bytes(b) => Ok(Value::Int(b.len() as i64)),
            Value::Dict(d) => Ok(Value::Int(d.borrow().len() as i64)),
            Value::Set(s) => Ok(Value::Int(s.borrow().len() as i64)),
            Value::FrozenSet(s) => Ok(Value::Int(s.len() as i64)),
            Value::Tuple(t) => Ok(Value::Int(t.len() as i64)),
            Value::PyObject(handle) => crate::interpreter::py_interop::py_len(handle),
            _ => Err(format!("TypeError: object of type '{}' has no len()", self.type_name(&val))),
        }
    }
}

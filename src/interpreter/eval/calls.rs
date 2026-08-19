// eval/calls.rs — 呼び出し評価: 関数呼び出し・キャスト・型コンストラクタ呼び出し・値呼び出し・AsyncManager 生成。


use crate::ast::Resolution;
use {
    std::cell::RefCell, std::rc::Rc, std::sync::Arc,
    crate::ast::{CallArg, Expr},
    crate::token::Span,
    crate::interpreter::{
        Interpreter, NativeFnRef, SliceValue, Value, Var,
    },
};
use super::*;

impl Interpreter {
    /// `obj.attr(...)` の呼び先が外部言語なら、その言語タグを返す（FFI 境界検査用）。
    ///
    /// 名前空間のメンバを読むだけで副作用は無いので、実際の呼び出し前に安全に判定できる。
    /// 新しい言語を足すときは、その言語の関数を表す `Value` 変種をここに 1 行足す。
    pub(crate) fn foreign_call_lang(obj: &Value, attr: &str) -> Option<&'static str> {
        match obj {
            // Python オブジェクトのメソッド呼び出し。
            Value::PyObject(_) => Some("py"),
            // モジュール名前空間経由（`mod.func()`）。メンバの種別で言語が決まる。
            Value::Namespace(ns) => match ns.members.get(attr) {
                Some(Value::PyObject(_)) => Some("py"),
                Some(Value::JsProcFn(_)) => Some("js-proc"),
                _ => None,
            },
            _ => None,
        }
    }

    /// FFI 境界検査（#16）: 外部言語の呼び出し結果が Arrow へ入る直前に、
    /// **スタブが宣言した戻り値型**（＝型検査がこの Call ノードへ焼いた解決型）と突き合わせる。
    ///
    /// 検査器は言語ごと（[`ffi_boundary::checker_for`]）。未対応言語・未採番ノード・
    /// 宣言型が無い（`Any`/`Unresolved`）場合は**素通し**なので、スタブが無い箇所の挙動は変わらない。
    /// 逆にスタブが型を宣言しているほど検査が効く。
    /// `callee_name` は表示用（エラーメッセージ）。**`&Expr` を取らない**のは、
    /// C 軸ディスパッチ（`call_value_evaled`）からも呼べるようにするため（#22-b）。
    pub(crate) fn check_ffi_return(
        &mut self,
        lang: &str,
        value: Value,
        node_id: u32,
        callee_name: &str,
        call_span: Option<&Span>,
    ) -> Result<Value, String> {
        use crate::interpreter::ffi_boundary::{checker_for, Verdict};

        let Some(checker) = checker_for(lang) else {
            return Ok(value);
        };
        let Some(declared) = self.annotations.resolved_type(node_id).cloned() else {
            return Ok(value);
        };

        match checker.check(&value, &declared) {
            Verdict::Ok | Verdict::Unverifiable => Ok(value),
            Verdict::Coerce(v) => Ok(v),
            Verdict::Mismatch { actual } => {
                let where_ = call_span.map_or_else(|| "<unknown>".to_string(), |s| s.to_string());
                let msg = format!(
                    "FfiTypeError: {} function `{}` is declared to return `{}` but returned `{}` at {}",
                    checker.lang(),
                    callee_name,
                    declared,
                    actual,
                    where_
                );
                match self.make_internal_raised_error(&msg) {
                    Some(raised) => {
                        self.current_exception = Some(raised);
                        Err(crate::interpreter::RAISE_SENTINEL.to_string())
                    }
                    None => Err(msg),
                }
            }
        }
    }

    /// 関数呼び出し式 `func(args)` を評価する。
    /// テンプレート instantiate・メソッド呼び出し・組み込み関数・ユーザー定義関数・クラスコンストラクタ・
    /// ジェネレータ・ネイティブ関数・型コンストラクタなど、呼び出し先の種別に応じて適切なパスへ分岐する。
    ///
    /// `node_id` は AST 型解決層の注釈を引くキー（#16）。FFI 境界検査が
    /// 「スタブがこの呼び出しの戻り値をどう宣言しているか」を知るために使う。0 = 未採番。
    pub(crate) fn eval_call(
        &mut self,
        func: &Expr,
        args: &[CallArg],
        call_span: &Span,
        cache: &crate::ast::NativeCallCache,
        node_id: u32,
    ) -> Result<Value, String> {
        // #55: AST 式を取るツリーウォーク入口の通過を数える（既定ビルドでは消える）。
        crate::interpreter::tw_stats::record_site(0);
        // ── インラインキャッシュ命中: AST に焼き込まれた typed ネイティブ関数 ──
        // スコープ検索・Value マッチ・組み込みチェックをすべて跳ばして直接ディスパッチ。
        // （充填条件: 不変バインディング + typed ABI あり — 下の NativeFunction アーム参照）
        let cached = cache.0.borrow().clone();
        if let Some(any_arc) = cached {
            if let Some(fn_ref) = any_arc.downcast_ref::<NativeFnRef>() {
                return self.dispatch_native_typed_exprs(fn_ref, &any_arc, args);
            }
        }

        if let Expr::TemplateInstantiate { base, type_args } = func {
            let tmpl_val = self.eval(base)?;
            return self.instantiate_template(tmpl_val, type_args, args);
        }
        if let Expr::Attr { object, attr, .. } = func {
            let obj_val = self.eval(object)?;
            // `mod.func()` 形式の外部言語呼び出しはここを通る（`eval_method_call` へ委譲される）。
            // 委譲先の署名を増やさずに済むよう、**呼ぶ前に**呼び先の言語を覗いておく。
            let lang = Self::foreign_call_lang(&obj_val, attr);
            let r = self.eval_method_call(obj_val, attr, args, Some(cache))?;
            return match lang {
                Some(l) => {
                    let n = callee_display_name(func);
                    self.check_ffi_return(l, r, node_id, &n, Some(call_span))
                }
                None => Ok(r),
            };
        }
        // ── R4: Arrow 関数呼び先キャッシュ命中（Ident のみ） ──
        // 不変グローバル関数と初回解決済みなら、builtin 判定・名前引き・name.clone を跳ばして
        // `scopes[0]` の slot から直接ディスパッチする。
        if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func {
            if let Some(idx) = cache.1.get(self.slot_epoch) {
                let cached_fn = match self.scopes[0].slot(idx) {
                    Some(Var::Immutable(Value::Function(f))) => Some(f.clone()),
                    _ => None,
                };
                if let Some(f) = cached_fn {
                    #[cfg(debug_assertions)]
                    {
                        // キャッシュした呼び先が、名前引き解決と一致することを検証する。
                        let live = self.get_val(name);
                        debug_assert!(
                            matches!(&live, Some(Value::Function(lf)) if Rc::ptr_eq(lf, &f)),
                            "R4 callee cache mismatch for '{name}'"
                        );
                    }
                    return self.exec_fn(f, args, None, name, Some(call_span.clone()));
                }
                // 想定外（束縛が変わった等）は通常経路へ委譲する。
            }
        }

        // ── 組み込みへの振り分け（#15d）──
        // `Resolution::Local`/`Global` は「リゾルバがユーザー変数と解決済み」＝組み込みではないので、
        // ここへは来ない（`Unresolved` に限定してある）。
        //
        // ただし **`Unresolved` は「シャドウが無い」ことを意味しない**。リゾルバが処理するのは
        // トップレベル関数の本体だけで、**モジュール最上位・テンプレート本体・合成 AST は常に
        // `Unresolved`** になる。そこで `let repr = my_fn` としても、名前だけで振り分けていた
        // 従来コードは組み込みを呼んでしまっていた（実測: print/next/zip/enumerate/getenv/repr の 6 件）。
        //
        // よってスコープに束縛があるかを見てから振り分ける。VM の
        // `is_vm_builtin(name) && !slots.contains_key(name)`（[vm/compiler.rs](../../vm/compiler.rs)）と同じ規則で、
        // これでツリーウォークと VM の健全性が揃う。
        // #22-d: `res` を条件から外した。組み込み名は AST で宣言される名前ではないため
        // リゾルバの `globals` に載らず、`res` は常に `Unresolved` になる＝**条件が実質無条件**で、
        // 「`res` を見て組み込みか判定している」と読めてしまう misleading な依存だった。
        // 判定の根拠は `builtin_is_shadowed`（実際の束縛）だけで足りる（#15e の原則）。
        // ユーザーが `let repr = f` と最上位で宣言した場合、関数本体からの参照は
        // `res == Global` になるが、`builtin_is_shadowed` が束縛を見つけるので結果は同じ。
        if let Expr::Ident { name, res, .. } = func {
            // `res` が解決済み ⟹ **必ずユーザーの束縛がある** ので組み込みではない。
            // これは意味論の判定ではなく、`builtin_is_shadowed` と**同値な高速パス**:
            // リゾルバが `Local`/`Global` を付けるのは AST で宣言された名前だけで、
            // 組み込み名は宣言できない（`let len = ...` は「already declared」で弾かれる）。
            // ＝ `res` が解決済みなら `builtin_is_shadowed(name)` は必ず true。
            //
            // #21-b で最上位の識別子が `Global` になったため、この高速パスが無いと
            // **最上位の全呼び出しで `builtin_is_shadowed` のスコープ走査が走る**。
            // （なお `bench_field_access` の 1.05→1.11s は 21-b 無効化でも再現したので
            //   21-b 由来ではない。この分岐は退行の修復ではなく純粋な無駄取り。）
            let resolved = !matches!(res, Resolution::Unresolved);
            if !resolved && !self.builtin_is_shadowed(name) {
                if let Some(result) = self.eval_builtin_ident_call(name, args) {
                    return result;
                }
            }
        }
        // トレースバック表示名（#15d）。`res` を問わず識別子の名前を使う。
        // 以前は `Unresolved` に限っていたため、**リゾルバが解決した呼び先＝関数本体からの呼び出しが
        // すべて `<anonymous>` になっていた**（VM 経路は名前を出すので off/auto で出力が食い違っていた）。
        let call_name: &str = match func {
            Expr::Ident { name: n, .. } => n,
            _ => "<anonymous>",
        };
        let callee = self.eval(func)?;

        // ── C 軸（実行方式）へ委譲する（#22-b）──
        // 以前はここに 11 アームの match があり、`call_value_evaled` および
        // `eval_method_call` の `Namespace` アームと**別々のアーム集合**で三重化していた。
        // ずれが実バグになったため（#22-a: `JsProcFn` 欠落による off/auto 不一致）、
        // 実行方式の判断は `call_value_evaled` 1 本に寄せる。
        //
        // ここに残すのは **A 軸（同定）の後処理**と、**引数の式が要る呼び先**だけ:
        match callee {
            // A 軸: 不変グローバル関数への Ident 呼び出しなら global slot を焼き込む（R4）。
            // 焼き込みは「どこから呼ばれたか」を知っている呼び出し側の責務なので C 軸へは移さない。
            Value::Function(fn_val) => {
                if let Expr::Ident { name, res: Resolution::Unresolved, .. } = func {
                    if !self.scopes[self.frame_floor..].iter().any(|s| s.contains_key(name)) {
                        if let Some(idx) = self.scopes[0].slot_of(name) {
                            if matches!(self.scopes[0].slot(idx), Some(Var::Immutable(_))) {
                                cache.1.fill(self.slot_epoch, idx as u32);
                            }
                        }
                    }
                }
                self.exec_fn(fn_val, args, None, call_name, Some(call_span.clone()))
            }
            // **C 軸へ渡せない唯一の呼び先**: ネイティブ関数。
            // `mut` ポインタ引数の write-back は「どの変数に書き戻すか」を知る必要があり、
            // 評価済みの値だけでは判定できない（`dispatch_native_evaled` は書き戻しをしない）。
            // typed IC の焼き込み（A 軸）も引数が全て位置引数かを式で見るためここに置く。
            Value::NativeFunction(fn_ref) => {
                if fn_ref.typed_sig.is_some()
                    && fn_ref.typed_fn_ptr.load(std::sync::atomic::Ordering::Relaxed) != 0
                    && fn_ref.n_params <= 16
                    && args.len() == fn_ref.n_params
                    && args.iter().all(|a| matches!(a, CallArg::Positional(_)))
                {
                    if let Expr::Ident { name, .. } = func {
                        let immutable_binding =
                            self.get_var(name).map(|v| !v.is_mutable()).unwrap_or(false);
                        if immutable_binding {
                            *cache.0.borrow_mut() =
                                Some(fn_ref.clone() as Arc<dyn std::any::Any + Send + Sync>);
                        }
                    }
                }
                self.call_native_function(&fn_ref, args)
            }
            // 残り全部（型コンストラクタ・クラス・ジェネレータ・オーバーロード・`__call__`・
            // py/js・テンプレート未実体化・非 callable）は C 軸が扱う。
            // `Value::Type` は 22-b では例外だったが、22-c で
            // `call_type_constructor_evaled`（キーワード名を保持）を用意して寄せた。
            other => {
                let evaled = self.eval_call_args(args)?;
                self.call_value_evaled(
                    other,
                    evaled,
                    call_name,
                    Some(call_span.clone()),
                    node_id,
                )
            }
        }
    }

    /// キャスト式 `obj => TypeName` を評価する。
    ///
    /// 動作:
    /// 1. ターゲット型が `new_type` クラスの場合 → コンストラクタ呼び出し `TypeName(inner_val)`
    ///    (obj 自身が new_type インスタンスのときは先に `.value` を取り出してからラップする)
    /// 2. obj が new_type インスタンスかつターゲット型がそのベース型の場合 → `.value` を返す
    /// 3. オブジェクトがインスタンスで `__cast__[TypeName]` メソッドを持つ場合 → そのメソッドを呼び出す
    /// 4. それ以外 → TypeError
    pub(crate) fn eval_cast(&mut self, object: &crate::ast::Expr, type_name: &str) -> Result<Value, String> {
        let obj = self.eval(object)?;
        self.eval_cast_evaled(obj, type_name)
    }

    /// 評価済みの値に対するキャスト `obj => TypeName`（VM の `Op::Cast` から使う）。
    /// `eval_cast` は被演算子を評価してからこれを呼ぶだけなので、両経路の意味論は同一。
    pub(crate) fn eval_cast_evaled(&mut self, obj: Value, type_name: &str) -> Result<Value, String> {

        // new_type インスタンスなら内部値を先に取り出しておく
        let inner_val = if let Value::Instance(ref inst_rc) = obj {
            let b = inst_rc.borrow();
            if b.class.new_type_base.is_some() {
                b.class.field_index.get("value").and_then(|&idx| b.field_value(idx))
            } else {
                None
            }
        } else {
            None
        };

        // --- new_type へのダウンキャスト: TypeName(obj) と等価 ---
        // obj 自身が new_type インスタンスの場合は内部値を渡してネストを防ぐ
        if let Some(target_val) = self.get_val(type_name) {
            if let Value::Class(ref cls) = target_val {
                if cls.new_type_base.is_some() {
                    let cls_rc = cls.clone();
                    let arg = inner_val.unwrap_or(obj);
                    return self.instantiate_evaled(cls_rc, vec![(None, arg, true)]);
                }
            }
        }

        // --- list ⇒ fixed_list: flat conversion ---
        let target_is_fixed = type_name == "fixed_list"
            || type_name.starts_with("fixed_list[");
        let target_is_list = type_name == "list"
            || type_name.starts_with("list[");
        if target_is_fixed {
            return match obj {
                Value::FrozenList { .. } => Ok(obj),  // already a fixed_list
                Value::List(ref rc) => {
                    let items = rc.borrow().clone();
                    Self::try_flat_freeze(&items).ok_or_else(|| {
                        "CastError: cannot cast list to fixed_list: \
                         elements must be homogeneous class instances \
                         with only int/float fields".to_string()
                    })
                }
                _ => Err(format!(
                    "CastError: cannot cast '{}' to 'fixed_list'",
                    self.type_name(&obj)
                )),
            };
        }
        if target_is_list {
            if let Value::FrozenList { ref state, ref layout } = obj {
                let st = state.borrow();
                let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                return Ok(Value::List(Rc::new(RefCell::new(items))));
            }
        }

        // --- インスタンスの __cast__[TypeName] メソッド呼び出し ---
        match &obj {
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();

                // new_type インスタンスをそのベース型にキャスト: .value を返す
                if let Some(ref base) = class.new_type_base {
                    if base == type_name {
                        let b = inst_rc.borrow();
                        let val = b.class.field_index.get("value").and_then(|&idx| b.field_value(idx));
                        return val.ok_or_else(|| {
                            format!("TypeError: '{}' has no 'value' field", class.name)
                        });
                    }
                }

                let method_key = format!("__cast__[{}]", type_name);
                let overloads = self
                    .lookup_method_in_class(&class, &method_key)
                    .ok_or_else(|| {
                        format!(
                        "TypeError: '{}' is not castable to '{}' (no __cast__[{}] method defined)",
                        class.name, type_name, type_name
                    )
                    })?;
                if overloads.len() == 1 {
                    self.exec_fn(overloads[0].clone(), &[], Some(obj), "__cast__", None)
                } else {
                    self.dispatch_overload(overloads, &[], Some(obj), None)
                }
            }
            other => Err(format!(
                "TypeError: cast operator '=>' requires an instance or new_type target, \
                 got '{}' cast to '{}'",
                self.type_name(other),
                type_name
            )),
        }
    }

    /// 型コンストラクタの評価済み引数版（#22-c）— **C 軸から呼べる形**。
    ///
    /// `call_type_by_name_evaled` と分けてあるのは、`AsyncManager` が**キーワード引数**を、
    /// `Signal[T]()` がテンプレート形を取り、どちらも「値のベクタ」では表現できないため。
    /// `evaled` はキーワード名（`Option<String>`）を保持しているのでここでは解ける。
    /// これにより `Value::Type` を `eval_call` の例外から外し、C 軸へ寄せられた。
    pub(crate) fn call_type_constructor_evaled(
        &mut self,
        type_name: &str,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        if type_name == "AsyncManager" {
            return self.make_async_manager_evaled(evaled);
        }
        // Signal[T]() — Arrow ネイティブのイベントソースを生成する。
        // テンプレート引数 T はランタイムでは無視する。
        if type_name == "Signal" {
            return Ok(Value::Signal(std::rc::Rc::new(std::cell::RefCell::new(
                crate::interpreter::event_loop::SignalData::new(),
            ))));
        }
        let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
        self.call_type_by_name_evaled(type_name, vals)
    }

    /// Dispatch an already-evaluated argument list to a built-in type constructor.
    /// Called by `call_type_constructor_evaled` and `call_value_with_args`
    /// (from native callbacks that hold a `Value::Type`).
    pub(crate) fn call_type_by_name_evaled(
        &mut self,
        type_name: &str,
        vals: Vec<Value>,
    ) -> Result<Value, String> {
        match type_name {
            "str" => {
                let has_instance_str = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__str__")
                } else {
                    false
                };
                if has_instance_str {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__str__", vec![])
                        .map(|r| match r {
                            Value::Str(s) => Value::Str(s),
                            other => Value::str(self.display(&other)),
                        });
                }
                match vals.as_slice() {
                    [] => Ok(Value::str("")),
                    [v] => Ok(Value::str(self.display(v))),
                    _ => Err("TypeError: str() takes at most 1 argument".to_string()),
                }
            },
            "int" => match vals.as_slice() {
                [] => Ok(Value::Int(0)),
                [Value::Int(n)] => Ok(Value::Int(*n)),
                [Value::Float(f)] => Ok(Value::Int(*f as i64)),
                [Value::Bool(b)] => Ok(Value::Int(if *b { 1 } else { 0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| format!("ValueError: invalid literal for int(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: int() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: int() takes at most 1 argument".to_string()),
            },
            "float" => match vals.as_slice() {
                [] => Ok(Value::Float(0.0)),
                [Value::Float(f)] => Ok(Value::Float(*f)),
                [Value::Int(n)] => Ok(Value::Float(*n as f64)),
                [Value::Bool(b)] => Ok(Value::Float(if *b { 1.0 } else { 0.0 })),
                [Value::Str(s)] => s
                    .trim()
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| format!("ValueError: invalid literal for float(): '{s}'")),
                [other] => Err(format!(
                    "TypeError: float() argument must be a string or a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: float() takes at most 1 argument".to_string()),
            },
            "complex" => match vals.as_slice() {
                [] => Ok(Value::Complex(0.0, 0.0)),
                [Value::Complex(re, im)] => Ok(Value::Complex(*re, *im)),
                [Value::Float(f)] => Ok(Value::Complex(*f, 0.0)),
                [Value::Int(n)] => Ok(Value::Complex(*n as f64, 0.0)),
                [Value::Complex(re, _), Value::Float(im2)] => Ok(Value::Complex(*re, *im2)),
                [Value::Float(re), Value::Float(im)] => Ok(Value::Complex(*re, *im)),
                [Value::Int(re), Value::Int(im)] => Ok(Value::Complex(*re as f64, *im as f64)),
                [Value::Int(re), Value::Float(im)] => Ok(Value::Complex(*re as f64, *im)),
                [Value::Float(re), Value::Int(im)] => Ok(Value::Complex(*re, *im as f64)),
                [other] => Err(format!(
                    "TypeError: complex() argument must be a number, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: complex() takes at most 2 arguments".to_string()),
            },
            "bool" => match vals.as_slice() {
                [] => Ok(Value::Bool(false)),
                [Value::Bool(b)] => Ok(Value::Bool(*b)),
                [Value::Int(n)] => Ok(Value::Bool(*n != 0)),
                [Value::Float(f)] => Ok(Value::Bool(*f != 0.0)),
                [Value::Str(s)] => Ok(Value::Bool(!s.is_empty())),
                [Value::None] => Ok(Value::Bool(false)),
                [Value::List(lst)] => Ok(Value::Bool(!lst.borrow().is_empty())),
                [Value::Set(s)] => Ok(Value::Bool(!s.borrow().is_empty())),
                [_] => Ok(Value::Bool(true)),
                _ => Err("TypeError: bool() takes at most 1 argument".to_string()),
            },
            "list" => match vals {
                ref v if v.is_empty() => Ok(Value::List(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => match vals.into_iter().next().unwrap() {
                    Value::List(lst) => Ok(Value::List(lst)),
                    Value::FrozenList { state, layout } => {
                        let st = state.borrow();
                        let items = (0..st.len).map(|i| layout.reconstruct_item(&st.data, i)).collect();
                        Ok(Value::List(Rc::new(RefCell::new(items))))
                    }
                    Value::Set(s) => Ok(Value::List(Rc::new(RefCell::new(s.borrow().clone())))),
                    Value::Str(s) => {
                        let chars = s.chars().map(|c| Value::str(c.to_string())).collect();
                        Ok(Value::List(Rc::new(RefCell::new(chars))))
                    }
                    other => Err(format!(
                        "TypeError: '{}' object is not iterable",
                        self.type_name(&other)
                    )),
                },
                _ => Err("TypeError: list() takes at most 1 argument".to_string()),
            },
            "set" => match vals {
                ref v if v.is_empty() => Ok(Value::Set(Rc::new(RefCell::new(vec![])))),
                _ if vals.len() == 1 => {
                    let arg = vals.into_iter().next().unwrap();
                    let items: Vec<Value> = match arg {
                        Value::Set(s) => s.borrow().clone(),
                        Value::List(lst) => lst.borrow().clone(),
                        Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
                        Value::Tuple(t) => t.all_values().to_vec(),
                        other => {
                            return Err(format!(
                                "TypeError: '{}' object is not iterable",
                                self.type_name(&other)
                            ))
                        }
                    };
                    let mut result: Vec<Value> = Vec::new();
                    for v in items {
                        set_insert(&mut result, v, self);
                    }
                    Ok(Value::Set(Rc::new(RefCell::new(result))))
                }
                _ => Err("TypeError: set() takes at most 1 argument".to_string()),
            },
            "slice" => {
                let check_index = |v: Value, label: &str| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        Value::Instance(ref inst) if inst.borrow().class.name == "Index" => {
                            Ok(Some(v))
                        }
                        other => Err(format!(
                            "TypeError: slice {label} must be int, Index, or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                let check_step = |v: Value| -> Result<Option<Value>, String> {
                    match v {
                        Value::None => Ok(None),
                        Value::Int(_) => Ok(Some(v)),
                        other => Err(format!(
                            "TypeError: slice step must be int or None, got '{}'",
                            self.type_name(&other)
                        )),
                    }
                };
                match vals.len() {
                    2 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        Ok(Value::Slice(Rc::new(SliceValue {
                            begin,
                            end,
                            step: None,
                        })))
                    }
                    3 => {
                        let mut it = vals.into_iter();
                        let begin = check_index(it.next().unwrap(), "begin")?;
                        let end = check_index(it.next().unwrap(), "end")?;
                        let step = check_step(it.next().unwrap())?;
                        Ok(Value::Slice(Rc::new(SliceValue { begin, end, step })))
                    }
                    _ => Err("TypeError: slice() takes 2 or 3 arguments".to_string()),
                }
            }
            "uint" => match vals.as_slice() {
                [] => Ok(Value::UInt(0)),
                [Value::UInt(n)] => Ok(Value::UInt(*n)),
                [Value::Int(n)] => Ok(Value::UInt(*n as u64)),
                [Value::Bool(b)] => Ok(Value::UInt(if *b { 1 } else { 0 })),
                [other] => Err(format!(
                    "TypeError: uint() argument must be an integer, not '{}'",
                    self.type_name(other)
                )),
                _ => Err("TypeError: uint() takes at most 1 argument".to_string()),
            },
            "id" => {
                if vals.len() != 1 {
                    return Err("TypeError: id() takes exactly one argument".to_string());
                }
                let val = vals.into_iter().next().unwrap();
                let raw: u64 = match &val {
                    Value::Instance(rc) => Rc::as_ptr(rc) as u64,
                    Value::List(rc) => Rc::as_ptr(rc) as u64,
                    Value::Dict(rc) => Rc::as_ptr(rc) as u64,
                    Value::Function(rc) => Rc::as_ptr(rc) as u64,
                    Value::OverloadedFn(v) => v.as_ptr() as u64,
                    Value::Generator(rc) => Rc::as_ptr(rc) as u64,
                    Value::GeneratorFn(rc) => Rc::as_ptr(rc) as u64,
                    Value::Tuple(rc) => Rc::as_ptr(rc) as u64,
                    Value::Int(n) => *n as u64,
                    Value::UInt(n) => *n,
                    Value::Float(f) => f.to_bits(),
                    Value::Bool(b) => *b as u64,
                    Value::Str(s) => {
                        use std::hash::{Hash, Hasher};
                        let mut h = std::collections::hash_map::DefaultHasher::new();
                        s.hash(&mut h);
                        h.finish()
                    }
                    Value::None => 0u64,
                    _ => 0u64,
                };
                let pointer_cls = match self.get_val("pointer") {
                    Some(Value::Class(cls)) => cls,
                    _ => return Err("RuntimeError: 'pointer' type is not defined".to_string()),
                };
                let mut inst = crate::interpreter::InstanceData::new_empty(pointer_cls, 0);
                inst.store_field(0, Value::UInt(raw), true);
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            }
            "len" => {
                let has_instance_len = if let [Value::Instance(inst_rc)] = vals.as_slice() {
                    inst_rc.borrow().class.methods.contains_key("__len__")
                } else {
                    false
                };
                if has_instance_len {
                    let v = vals.into_iter().next().unwrap();
                    return self.eval_method_call_evaled(v, "__len__", vec![])
                        .and_then(|r| match r {
                            Value::Int(n) => Ok(Value::Int(n)),
                            other => Err(format!(
                                "TypeError: __len__ must return int, not '{}'",
                                self.type_name(&other)
                            )),
                        });
                }
                match vals.as_slice() {
                    [Value::List(lst)] => Ok(Value::Int(lst.borrow().len() as i64)),
                    [Value::FrozenList { state, .. }] => Ok(Value::Int(state.borrow().len as i64)),
                    [Value::Str(s)] => Ok(Value::Int(s.len() as i64)),
                    [Value::Dict(d)] => Ok(Value::Int(d.borrow().len() as i64)),
                    [Value::Set(s)] => Ok(Value::Int(s.borrow().len() as i64)),
                    [Value::Tuple(t)] => Ok(Value::Int(t.len() as i64)),
                    [other] => Err(format!(
                        "TypeError: object of type '{}' has no len()",
                        self.type_name(other)
                    )),
                    _ => Err("TypeError: len() takes exactly 1 argument".to_string()),
                }
            },
            // Result コンストラクタ: Ok(value) / Err(error)
            "Ok" => match vals.as_slice() {
                [v] => Ok(Value::ResultVal { ok: true, inner: Box::new(v.clone()) }),
                _ => Err("TypeError: Ok() takes exactly 1 argument".to_string()),
            },
            "Err" => match vals.as_slice() {
                [v] => Ok(Value::ResultVal { ok: false, inner: Box::new(v.clone()) }),
                _ => Err("TypeError: Err() takes exactly 1 argument".to_string()),
            },
            other => Err(format!("TypeError: '{}' object is not callable", other)),
        }
    }

    // --- ネイティブ関数呼び出し ---

    /// 任意の呼び出し可能な `Value` を評価済み引数リストで呼び出す。
    /// ネイティブコールバック `ar_call_fn` から呼ばれる。
    pub(crate) fn call_value_with_args(
        &mut self,
        callee: Value,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        // ネイティブ呼び出しは引数を保守的に mutable 扱い（従来動作）。
        let evaled: Vec<(Option<String>, Value, bool)> =
            args.into_iter().map(|v| (None, v, true)).collect();
        self.call_value_evaled(callee, evaled, "<fn>", None, 0)
    }

    /// 評価済み引数（`is_mutable` フラグ込み）で任意の呼び出し可能値をディスパッチする。
    /// VM の `Call` op（正しい `is_mutable` フラグをコンパイル時に算出）と `call_value_with_args`
    /// の共通実装。
    /// **C 軸（実行方式）の唯一の実装**（#22-b）。
    ///
    /// 呼び先の値を 4 分類のどれかで実行する:
    ///
    /// - **組み込み**: `Type`（型コンストラクタ）
    /// - **非コンパイルの Arrow 関数・メソッド**: `Function` / `OverloadedFn` / `Class` /
    ///   `GeneratorFn` / `Instance`（`__call__`）
    /// - **C ABI シンボル**: `NativeFunction`。Arrow をコンパイルしたものと外部ライブラリは同一表現
    ///   （違いは出自であって呼び出し経路ではない）
    /// - **翻訳機経由**: `PyObject` / `JsProcFn`（C# はブリッジ側で処理）
    ///
    /// 以前はこのディスパッチが `eval_call` / ここ / `eval_method_call` の `Namespace` アームの
    /// **3 箇所に別々のアーム集合で**存在し、ずれが実バグになっていた（#22-a で `JsProcFn` 欠落による
    /// off/auto 不一致を検出）。呼び先の同定（A 軸）と正規化（B 軸）は呼び出し側の責務とし、
    /// ここは「値を受け取って実行方式を選ぶ」だけに保つこと。
    ///
    /// `node_id` は FFI 境界検査（#16）が宣言型を引くキー。**0 = 未採番＝検査しない**。
    /// これを運ばないと VM 経路で境界検査が丸ごと効かなくなる（#22-a 発見 2）。
    ///
    /// ⚠ `NativeFunction` の **write-back（`mut` ポインタ引数の書き戻し）だけは
    /// 引数の式が要る**ため、ここには来ず `eval_call` 側の `call_native_function` が扱う。
    /// この経路（評価済み引数）は書き戻しを行わない — 判定不能なので安全側に倒している。
    pub(crate) fn call_value_evaled(
        &mut self,
        callee: Value,
        evaled: Vec<(Option<String>, Value, bool)>,
        fn_name: &str,
        call_span: Option<Span>,
        node_id: u32,
    ) -> Result<Value, String> {
        match callee {
            Value::Function(fn_val) => {
                self.exec_fn_evaled(fn_val, &evaled, None, fn_name, call_span)
            }
            Value::OverloadedFn(candidates) => {
                self.dispatch_overload_evaled(candidates, evaled, None, fn_name, call_span)
            }
            Value::Class(cls) => self.instantiate_evaled(cls, evaled),
            // ジェネレータ関数呼び出し（VM の Call op 用）。ツリーウォークの `eval_call` の
            // `GeneratorFn => exec_generator` に対応。本体は eager 実行し `Value::Generator` を返す。
            Value::GeneratorFn(gen_fn) => self.exec_generator_evaled(gen_fn, evaled, None),
            Value::NativeFunction(fn_ref) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                self.dispatch_native_evaled(&fn_ref, vals)
            }
            // #22-c: キーワード引数を保持したまま渡すので `AsyncManager`/`Signal` も扱える。
            Value::Type(type_name) => self.call_type_constructor_evaled(&type_name, evaled),
            Value::Instance(_) => self.eval_method_call_evaled(callee, "__call__", evaled),
            Value::PyObject(ref handle) => {
                let r = crate::interpreter::py_interop::call_py_object(handle, &evaled)?;
                self.check_ffi_return("py", r, node_id, fn_name, call_span.as_ref())
            }
            // #22-a で欠落が判明したアーム。`eval_call` には有ったが**ここには無く**、
            // VM の `Op::Call` はこの関数を使うため、`let f = js_mod.func` を
            // VM 化された関数内から呼ぶと `'function' object is not callable` で落ちていた
            // （当時は `--vm=off` では通る＝off/auto の不一致だった。`--vm` は #33 で削除）。
            //
            // ⚠ 残る差: ツリーウォーク側は続けて `check_ffi_return` で戻り値を検査するが、
            // 検査は**型検査が Call ノードへ焼いた宣言型**（node_id 索引）を根拠にするため、
            // node_id を持たないこの経路では行えない。`PyObject` アームも同じ状態で、
            // **VM 経路では FFI 境界検査が効いていない**。C 軸を 1 本化する #22-b で解消する。
            Value::JsProcFn(ref data) => {
                let vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
                let r = crate::interpreter::js_proc_runtime::call_function(
                    &data.bridge_key,
                    &data.module_name,
                    &data.fn_name,
                    &vals,
                )?;
                self.check_ffi_return("js-proc", r, node_id, fn_name, call_span.as_ref())
            }
            // エラー文言も 1 箇所に揃える（以前は `eval_call` にしか無かった）。
            Value::Protocol(ref proto_name) => Err(format!(
                "TypeError: protocol '{proto_name}' cannot be instantiated"
            )),
            Value::TemplateFn(_) | Value::TemplateClass(_) | Value::TemplateGenFn(_) => Err(
                "TemplateError: template must be called with explicit type arguments (e.g. `Func[T](args)`)".to_string()
            ),
            other => Err(format!(
                "TypeError: '{}' object is not callable",
                self.type_name(&other)
            )),
        }
    }

    /// グローバルスコープ（`scopes[0]`）での名前 → slot 番号（#11: VM の LoadGlobal 索引化）。
    /// 呼び出し元スコープを跨がず、トップレベル関数の自由名＝グローバルという規則に一致する。
    /// グローバルは追記のみで index は安定。`LoadGlobal` の runtime index cache 充填に使う。
    pub(crate) fn vm_global_slot_of(&self, name: &str) -> Option<usize> {
        self.scopes[0].slot_of(name)
    }

    /// グローバル slot 番号から値を読む（`vm_global_slot_of` で解決した index の再利用）。
    pub(crate) fn vm_global_by_slot(&self, idx: usize) -> Option<Value> {
        self.scopes[0].slot(idx).map(|v| v.get_value())
    }

    /// `slot_epoch`（`freeze` で進む）。VM の LoadGlobal cache の世代検証に使う。
    pub(crate) fn vm_slot_epoch(&self) -> u32 {
        self.slot_epoch
    }

    /// AsyncManager(num_thread=N [, raise_immediately=bool]) コンストラクタ
    /// `AsyncManager(...)` の評価済み引数版（#22-c）。
    /// キーワード引数（`num_thread=` / `raise_immediately=`）を見るので、
    /// **キーワード名を保持した `evaled` を受け取る**必要がある。
    pub(crate) fn make_async_manager_evaled(
        &mut self,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        let mut num_thread: usize = 1;
        let mut raise_immediately: bool = false;

        match evaled.as_slice() {
            [] => {}
            _ => {
                for (kw, val, _) in &evaled {
                    match kw.as_deref() {
                        Some("num_thread") | None => {
                            match val {
                                Value::Int(n) if *n > 0 => num_thread = *n as usize,
                                Value::UInt(n) => num_thread = *n as usize,
                                other => return Err(format!(
                                    "TypeError: AsyncManager num_thread must be a positive int, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some("raise_immediately") => {
                            match val {
                                Value::Bool(b) => raise_immediately = *b,
                                other => return Err(format!(
                                    "TypeError: AsyncManager raise_immediately must be bool, got '{}'",
                                    self.type_name(other)
                                )),
                            }
                        }
                        Some(k) => return Err(format!(
                            "TypeError: AsyncManager() got unexpected keyword argument '{k}'"
                        )),
                    }
                }
            }
        }

        let mgr = crate::interpreter::async_mgr::AsyncManagerData::new(num_thread, raise_immediately);
        Ok(Value::AsyncManager(Rc::new(RefCell::new(mgr))))
    }
}

/// 呼び先の表示名（FFI 境界検査のエラーメッセージ用）。`mod.fn` 形式を優先する。
fn callee_display_name(func: &Expr) -> String {
    match func {
        Expr::Ident { name: n, .. } => n.clone(),
        Expr::Attr { object, attr, .. } => match object.as_ref() {
            Expr::Ident { name: base, .. } => format!("{base}.{attr}"),
            _ => attr.clone(),
        },
        _ => "<callee>".to_string(),
    }
}

// templates.rs — テンプレート展開・AST置換
// (check_template_constraints / type_satisfies_trait / instantiate_template / instantiate_template_class)
// + subst_* フリー関数 (AST substitution helpers for template instantiation)
//
// テンプレート関数・クラス・ジェネレータ関数の呼び出し時に型変数を具体型に置換して実行する。
// `subst_*` フリー関数群が AST ノードを再帰的に走査して型変数名を書き換える。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{CallArg, FieldKind, Stmt, TemplateParam};

// 置換は `crate::template_subst`（タスク 2-7 で切り出した・実行時と展開時で同じものを使う）。
use crate::template_subst::{subst_params, subst_stmts, subst_type};

use super::{
    ClassValue, DictData, FnValue, GeneratorFnValue, Interpreter, TemplateClassValue, Value,
};

/// テンプレート実体化に渡す呼び出し引数（#27-c）。
///
/// ツリーウォークは AST のまま、VM はスタックから取った評価済み値を渡す。**評価の時点を
/// ずらさない**ために、`instantiate_template_args` は最後まで未評価のまま持ち回り、
/// 呼び先へ渡す直前に `into_evaled` する。
pub(crate) enum TemplateArgs<'a> {
    Ast(&'a [CallArg]),
    Evaled(Vec<(Option<String>, Value, bool)>),
}

impl TemplateArgs<'_> {
    fn into_evaled(
        self,
        it: &mut Interpreter,
    ) -> Result<Vec<(Option<String>, Value, bool)>, String> {
        match self {
            TemplateArgs::Ast(args) => it.eval_call_args(args),
            TemplateArgs::Evaled(v) => Ok(v),
        }
    }
}

impl Interpreter {
    /// 各具体型引数がテンプレートパラメータの trait 制約を満たすか検証する。
    ///
    /// - `template_params`: テンプレートパラメータリスト（型変数名と制約）
    /// - `type_args`: 呼び出し時に渡された具体型名のリスト
    ///
    /// 戻り値: `Ok(())` — すべての制約を満たす。`Err(message)` — 型引数数不一致または制約違反
    pub(super) fn check_template_constraints(
        &self,
        template_params: &[TemplateParam],
        type_args: &[String],
    ) -> Result<(), String> {
        if template_params.len() != type_args.len() {
            return Err(format!(
                "TemplateError: expected {} type argument(s), got {}",
                template_params.len(),
                type_args.len()
            ));
        }
        // 各型変数とその具体型を対応付けて制約を検証する
        for (param, type_name) in template_params.iter().zip(type_args.iter()) {
            for constraint in &param.constraints {
                if !self.type_satisfies_trait(type_name, constraint)? {
                    return Err(format!(
                        "TemplateError: type `{type_name}` does not satisfy trait `{constraint}` \
                         (required for template parameter `{}`)",
                        param.name
                    ));
                }
            }
        }
        Ok(())
    }

    /// 指定した型名が trait を実装しているか（`bases` に含まれているか）を返す。
    ///
    /// 組み込み型（`int`, `str` 等）は trait を実装していないため常に `false` を返す。
    ///
    /// - `type_name`: 検査する型の名前（スコープから検索される）
    /// - `trait_name`: 実装されているか確認する trait 名
    ///
    /// 戻り値: `Ok(true)` — 実装あり、`Ok(false)` — 実装なし、`Err` — 型が未定義
    pub(super) fn type_satisfies_trait(
        &self,
        type_name: &str,
        trait_name: &str,
    ) -> Result<bool, String> {
        match self.get_val(type_name) {
            Some(Value::Class(cls)) => Ok(cls.bases.contains(&trait_name.to_string())),
            Some(_) => Ok(false), // 組み込み型や非クラス値は trait を実装していない
            None => Err(format!("NameError: type `{type_name}` is not defined")),
        }
    }

    /// テンプレート関数・クラス・ジェネレータを型引数で実体化して実行または構築する。
    ///
    /// ディスパッチ先:
    /// - `TemplateFn`: 型変数を置換して通常関数として実行
    /// - `TemplateClass`: 型変数を置換してクラスを構築してインスタンス化
    /// - `TemplateGenFn`: 型変数を置換してジェネレータ関数として実行
    /// - `Type("dict")`: `dict[K, V](...)` の組み込み辞書コンストラクタとして処理
    ///
    /// - `tmpl_val`: 実体化するテンプレート値
    /// - `type_args`: 具体型名のリスト（テンプレートパラメータと同数）
    /// - `call_args`: 実体化後の関数/コンストラクタ呼び出し引数（AST または評価済み）
    ///
    /// 戻り値: `Ok(Value)` — 実行結果またはインスタンス。`Err(message)` — 制約違反・型エラー等
    ///
    /// ⚠ 引数は `TemplateArgs` のまま持ち回り、**各分岐が呼び先へ渡す直前**に評価する。
    /// 先に評価してしまうと、制約検証（`check_template_constraints`）より前に引数の
    /// 副作用が起きてツリーウォークと順序がずれる（#27-c）。
    pub(super) fn instantiate_template(
        &mut self,
        tmpl_val: Value,
        type_args: &[String],
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        self.instantiate_template_args(tmpl_val, type_args, TemplateArgs::Ast(call_args))
    }

    /// 評価済み引数でテンプレートを実体化する（VM の `Op::CallTemplate` 用・#27-c）。
    /// `instantiate_template` と**同一の本体**を通る。
    pub(crate) fn instantiate_template_evaled(
        &mut self,
        tmpl_val: Value,
        type_args: &[String],
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        self.instantiate_template_args(tmpl_val, type_args, TemplateArgs::Evaled(evaled))
    }

    fn instantiate_template_args(
        &mut self,
        tmpl_val: Value,
        type_args: &[String],
        call_args: TemplateArgs<'_>,
    ) -> Result<Value, String> {
        match tmpl_val {
            Value::TemplateFn(tmpl) => {
                // テンプレート関数: 制約を検証し、型変数を具体型に置換して通常関数として実行する。
                // 制約検証は毎回行う（安価・エラー意味論を保つ）。AST 置換と FnValue 構築は
                // `(テンプレート, 型引数)` でメモ化し、再実体化で clone-walk と Chunk 再コンパイルを省く（#7）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ **展開器が単相化した宣言は、定義された時点でこのキャッシュに入っている**
                //   （`register_mono_instance`・タスク 2-15）。無いとき（REPL・デバッガ・
                //   制約を確かめられなかった具体化）だけここで作る。
                let fn_val = match self.template_fn_cache.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let type_map: HashMap<String, String> = tmpl
                            .template_params
                            .iter()
                            .zip(type_args.iter())
                            .map(|(p, t)| (p.name.clone(), t.clone()))
                            .collect();
                        let concrete_params = subst_params(&tmpl.params, &type_map);
                        let concrete_body = subst_stmts(&tmpl.body, &type_map);
                        let fn_val = Rc::new(FnValue {
                            name: tmpl.name.clone(),
                            params: concrete_params,
                            body: std::rc::Rc::from(concrete_body),
                            is_python: false,
                            captured_env: std::collections::HashMap::new(),
                            // 0-B2 の残件: 戻り値注釈も具体型へ置換して載せる。
                            // 無いと `fn f[T](…) -> float` の戻り値昇格が効かない。
                            return_type: tmpl
                                .return_type
                                .as_ref()
                                .map(|t| subst_type(t, &type_map)),
                            vm_chunk: None,
                        });
                        self.template_fn_cache.insert(key, fn_val.clone());
                        fn_val
                    }
                };
                let evaled = call_args.into_evaled(self)?;
                self.exec_fn_evaled(fn_val, &evaled, None, "<template_fn>", None)
            }
            Value::TemplateClass(tmpl) => {
                // テンプレートクラス: 制約を検証し、型変数を置換してクラスを構築・インスタンス化する。
                //
                // ⚠ 制約検証は**キャッシュ引きの前に毎回**行う（関数側と同じ・エラー意味論を保つ）。
                //
                // ⚠⚠ **クラスは `(テンプレート, 型引数)` でメモ化する**（D3）。
                //    以前はキャッシュが無く、実体化のたびに `alloc_class_id()` で
                //    **新しい class_id** を発行していたため、同じ `Box[int]` を 2 回書くと
                //    `Value::Class` の等値（class_id 比較）が **False** になっていた。
                //    メモ化すると「同じ型引数で実体化したテンプレートは同じ型」になり、
                //    R3 の属性 IC も効くようになる（実体化ごとに class_id が変わると
                //    構造的に毎回ミスする）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ **展開器が単相化したクラスは、定義された時点でこのキャッシュに入っている**
                //   （`register_mono_instance`・タスク 2-15）。クラス変数の初期化などもその定義で済んでいる。
                let cls = match self.template_class_cache.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let type_map: HashMap<String, String> = tmpl
                            .template_params
                            .iter()
                            .zip(type_args.iter())
                            .map(|(p, t)| (p.name.clone(), t.clone()))
                            .collect();
                        // ⚠ 置換（clone-walk）も**ミス側**に置く。呼び出し元で先に置換すると
                        //   キャッシュを足しても毎回 AST を複製することになる（関数側と同じ形）。
                        let concrete_body = subst_stmts(&tmpl.body, &type_map);
                        let cls = self.build_template_class(&tmpl, concrete_body)?;
                        self.template_class_cache.insert(key, cls.clone());
                        cls
                    }
                };
                // ⚠ 引数の評価は**クラスが決まった後**（既存の順序を保つ。先に評価すると
                //   制約検証より前に副作用が起きる）。
                let evaled = call_args.into_evaled(self)?;
                self.instantiate_evaled(cls, evaled)
            }
            Value::TemplateGenFn(tmpl) => {
                // テンプレートジェネレータ関数: 型変数を置換してジェネレータとして実行する。
                // TemplateFn と同様に `(テンプレート, 型引数)` でメモ化（#7）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ 展開器が単相化したジェネレータは、定義された時点でこのキャッシュに入っている（2-15）。
                let gen_fn = match self.template_gen_cache.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let type_map: HashMap<String, String> = tmpl
                            .template_params
                            .iter()
                            .zip(type_args.iter())
                            .map(|(p, t)| (p.name.clone(), t.clone()))
                            .collect();
                        let concrete_params = subst_params(&tmpl.params, &type_map);
                        let concrete_body = subst_stmts(&tmpl.body, &type_map);
                        let gen_fn = Rc::new(GeneratorFnValue {
                            name: tmpl.name.clone(),
                            params: concrete_params,
                            body: concrete_body,
                            captured_env: std::collections::HashMap::new(),
                        });
                        self.template_gen_cache.insert(key, gen_fn.clone());
                        gen_fn
                    }
                };
                let evaled = call_args.into_evaled(self)?;
                self.exec_generator_evaled(gen_fn, evaled, None)
            }
            // Signal[T]() — 型付きシグナルを生成する（型引数は型チェックの注釈としてのみ使用）
            Value::Type(ref t) if t == "Signal" => {
                Ok(Value::Signal(std::rc::Rc::new(std::cell::RefCell::new(
                    super::event_loop::SignalData::new(),
                ))))
            }
            // 組み込み辞書型コンストラクタ: `dict[KeyType, ItemType](...)`
            Value::Type(ref t) if t == "dict" => {
                if type_args.len() != 2 {
                    return Err(format!(
                        "TypeError: dict requires exactly 2 type arguments [key_type, item_type], got {}",
                        type_args.len()
                    ));
                }
                let key_type = type_args[0].clone();
                let item_type = type_args[1].clone();

                let call_args = call_args.into_evaled(self)?;
                if call_args.is_empty() {
                    // `dict[K, V]()` — 空の型付き辞書を生成する
                    Ok(Value::Dict(Rc::new(RefCell::new(DictData::new(
                        key_type, item_type,
                    )))))
                } else if call_args.len() == 1 {
                    // `dict[K, V]({key: val, ...})` — 辞書リテラルから型付き辞書を生成する
                    let arg_val = call_args[0].1.clone();
                    match arg_val {
                        Value::Dict(src_rc) => {
                            let src = src_rc.borrow();
                            let src_keys = src.all_keys();
                            let src_vals = src.all_items();
                            // 各キーと値が宣言された型と一致するか検査する
                            for k in &src_keys {
                                if !Self::value_matches_type(k, &key_type) {
                                    return Err(format!(
                                        "StaticTypeError: dict key type mismatch: \
                                         expected '{}', got '{}'",
                                        key_type,
                                        self.type_name(k)
                                    ));
                                }
                            }
                            for v in &src_vals {
                                if !Self::value_matches_type(v, &item_type) {
                                    return Err(format!(
                                        "StaticTypeError: dict item type mismatch: \
                                         expected '{}', got '{}'",
                                        item_type,
                                        self.type_name(v)
                                    ));
                                }
                            }
                            // 型チェック通過後にソースデータをコピーして新しい型付き辞書を構築する
                            let mut new_data = DictData::new(key_type, item_type);
                            for (k, v) in src_keys.into_iter().zip(src_vals) {
                                Self::dict_set_pure(&mut new_data, k, v)?;
                            }
                            Ok(Value::Dict(Rc::new(RefCell::new(new_data))))
                        }
                        _ => Err(
                            "TypeError: dict constructor argument must be a dict literal `{...}`"
                                .to_string(),
                        ),
                    }
                } else {
                    Err("TypeError: dict constructor takes 0 or 1 argument".to_string())
                }
            }
            _ => Err("TemplateError: expression is not a template".to_string()),
        }
    }

    /// 型変数が置換されたテンプレートクラス本体から具体的な `ClassValue` を構築してインスタンス化する。
    ///
    /// `exec` の `Stmt::ClassDef` 処理と同様にクラス本体を走査してメソッド・フィールド・クラス変数を収集し、
    /// `ClassValue` を構築してから `instantiate` でインスタンスを生成する。
    ///
    /// - `tmpl`: 元のテンプレートクラス定義（名前・bases を参照する）
    /// - `concrete_body`: 型変数が具体型に置換済みのクラス本体文リスト
    /// - `call_args`: コンストラクタ呼び出し引数リスト（AST または評価済み）
    ///
    /// 戻り値: `Ok(Value::Instance)` — 構築済みインスタンス。`Err` — 実行エラー
    /// 置換済みの本体から**クラスそのもの**を構築する（インスタンス化はしない）。
    ///
    /// ⚠ ここには**副作用がある** — `const` / `static mut` / フィールド既定値の初期化子を
    /// `self.eval` する。呼び出し側が `(テンプレート, 型引数)` でメモ化するので、
    /// この評価は**その組み合わせにつき 1 回**になる。通常のクラス定義
    /// （`exec_class_def` は 1 回だけ走る）と同じ回数であり、以前の
    /// 「実体化のたびに評価し直す」方が非対称だった。
    /// 単相化した宣言（`Box[int]`）を定義したときに、**定義したスコープで見えるテンプレート**の
    /// 具体化としてキャッシュへ登録する（タスク 2-15）。
    ///
    /// ⚠⚠ 以前（2-8）は具体化の式のたびに**呼び出し側の名前** `Box[int]` を引いていた。メインと
    ///   `import` したモジュールが同じ名前のテンプレートを持つと、`m.Box[int](..)` が**メインの**
    ///   `Box[int]` を使っていた（実測）。テンプレートの値（の同一性）で結び付ければ取り違えない。
    ///   定義はテンプレートと同じ本体の最上位にあるので、ここで引く `Box` はそのテンプレート。
    pub(crate) fn register_mono_instance(&mut self, name: &str, value: &Value) {
        let Some((base, args)) = crate::template_subst::split_instance_name(name) else {
            return;
        };
        let tmpl = self
            .scopes
            .last()
            .and_then(|s| s.get(base))
            .map(|v| v.get_value())
            .or_else(|| self.vm_load_name(base));
        match (tmpl, value) {
            (Some(Value::TemplateClass(t)), Value::Class(c)) => {
                self.template_class_cache.insert((Rc::as_ptr(&t) as usize, args), c.clone());
            }
            (Some(Value::TemplateFn(t)), Value::Function(f)) => {
                self.template_fn_cache.insert((Rc::as_ptr(&t) as usize, args), f.clone());
            }
            (Some(Value::TemplateGenFn(t)), Value::GeneratorFn(g)) => {
                self.template_gen_cache.insert((Rc::as_ptr(&t) as usize, args), g.clone());
            }
            _ => {}
        }
    }

    pub(super) fn build_template_class(
        &mut self,
        tmpl: &TemplateClassValue,
        concrete_body: Vec<Stmt>,
    ) -> Result<Rc<ClassValue>, String> {
        let mut methods: HashMap<String, Vec<Rc<FnValue>>> = HashMap::new();
        let mut gen_methods: HashMap<String, Rc<GeneratorFnValue>> = HashMap::new();
        let mut field_defaults = Vec::new();
        let mut class_vars: HashMap<String, Value> = HashMap::new();
        let mut field_mutability: HashMap<String, bool> = HashMap::new();
        let mut field_access: HashMap<String, crate::ast::Accessibility> = HashMap::new();
        let mut method_access: HashMap<String, crate::ast::Accessibility> = HashMap::new();
        let mut static_method_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut class_method_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut static_vars: HashMap<String, Rc<RefCell<Value>>> = HashMap::new();
        let mut own_field_order: Vec<(String, bool, String)> = Vec::new();
        for stmt in &concrete_body {
            match stmt {
                Stmt::FnDef {
                    name: mname,
                    template_params,
                    params,
                    body: mbody,
                    access: macc,
                    is_static,
                    is_class_method,
                    ..
                } => {
                    let storage_name = if mname == "__cast__" && !template_params.is_empty() {
                        format!("__cast__[{}]", template_params[0].name)
                    } else {
                        mname.clone()
                    };
                    if *is_static {
                        static_method_names.insert(storage_name.clone());
                    }
                    if *is_class_method {
                        class_method_names.insert(storage_name.clone());
                    }
                    if *macc != crate::ast::Accessibility::Public {
                        method_access.insert(storage_name.clone(), macc.clone());
                    }
                    methods
                        .entry(storage_name)
                        .or_default()
                        .push(Rc::new(FnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: std::rc::Rc::from(&mbody[..]),
                            is_python: false,
                            captured_env: std::collections::HashMap::new(),
                        return_type: None,
                        vm_chunk: None,
                        }));
                }
                Stmt::GenDef {
                    name: mname,
                    params,
                    body: mbody,
                    access: macc,
                    ..
                } => {
                    if *macc != crate::ast::Accessibility::Public {
                        method_access.insert(mname.clone(), macc.clone());
                    }
                    gen_methods.insert(
                        mname.clone(),
                        Rc::new(GeneratorFnValue {
                            name: mname.clone(),
                            params: params.clone(),
                            body: mbody.clone(),
                            captured_env: std::collections::HashMap::new(),
                        }),
                    );
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::Const,
                    default: Some(init),
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = self.eval(init)?;
                    class_vars.insert(fname.clone(), val);
                }
                Stmt::Field {
                    name: fname,
                    kind: FieldKind::StaticMut,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let val = if let Some(init) = default {
                        self.eval(init)?
                    } else {
                        Value::None
                    };
                    static_vars.insert(fname.clone(), Rc::new(RefCell::new(val)));
                }
                Stmt::Field {
                    name: fname,
                    kind,
                    type_ann,
                    default,
                    access: facc,
                    ..
                } => {
                    if *facc != crate::ast::Accessibility::Public {
                        field_access.insert(fname.clone(), facc.clone());
                    }
                    let mutable = *kind == FieldKind::Mut;
                    field_mutability.insert(fname.clone(), mutable);
                    // ⚠ `type_ann` は `subst_stmts` が型変数を具体型へ置換済み
                    //   （`Box[int]` なら `"int"`）。実行時型判定タグの素になる。
                    own_field_order.push((fname.clone(), mutable, type_ann.clone()));
                    if let Some(init) = default {
                        let val = self.eval(init)?;
                        field_defaults.push((fname.clone(), val, mutable));
                    }
                }
                _ => {}
            }
        }
        let (field_index, field_mutability_vec, field_tags, field_checks, field_count) =
            self.build_field_index(&own_field_order, &tmpl.bases);
        let cls = Rc::new(ClassValue {
            bases: tmpl.bases.clone(),
            methods,
            gen_methods,
            field_defaults,
            class_vars,
            field_mutability,
            field_index,
            field_count,
            field_mutability_vec,
            field_tags,
            field_checks,
            field_access,
            method_access,
            static_method_names,
            class_method_names,
            static_vars,
            ..ClassValue::synthetic(tmpl.name.clone(), crate::interpreter::value::alloc_class_id())
        });
        Ok(cls)
    }
}

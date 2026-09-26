// templates.rs — テンプレートの具体化の呼び出し
// (check_template_constraints / type_satisfies_trait / instantiate_template / register_mono_instance)
//
// ⚠⚠ **具体化は展開時に作る**（D36・タスク 2-16）。展開器（`meta_expand::monomorph`）が
// 具体化を普通の宣言（`Box[int]`）として置き、それが定義された時点で `(テンプレート, 型引数)` の
// 組で登録される（`register_mono_instance`）。ここは呼び出しのたびに制約を確かめ、登録済みの
// 具体化を呼ぶだけで、型変数の置換（`crate::template_subst`）はしない。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{CallArg, TemplateParam};

use super::{DictData, Interpreter, Value};

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

    /// テンプレート関数・クラス・ジェネレータの具体化を呼ぶ（具体化そのものは展開時に作ってある）。
    ///
    /// ディスパッチ先:
    /// - `TemplateFn`: 登録済みの具体化（普通の関数）を実行
    /// - `TemplateClass`: 登録済みの具体化（普通のクラス）をインスタンス化
    /// - `TemplateGenFn`: 登録済みの具体化をジェネレータ関数として実行
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
                // テンプレート関数: 制約を検証し、展開時に作った具体化（普通の関数）を実行する。
                // 制約検証は毎回行う（安価・エラー意味論を保つ）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ **具体化は展開時に作る**（D36・タスク 2-16）。展開器が置いた宣言は、定義された
                //   時点でこのキャッシュに入る（`register_mono_instance`）。実行時には作らない。
                let Some(fn_val) = self.template_fn_cache.get(&key).cloned() else {
                    return Err(Self::not_instantiated(&tmpl.name, type_args));
                };
                let evaled = call_args.into_evaled(self)?;
                self.exec_fn_evaled(fn_val, &evaled, None, "<template_fn>", None)
            }
            Value::TemplateClass(tmpl) => {
                // テンプレートクラス: 制約を検証し、展開時に作った具体化（普通のクラス）をインスタンス化する。
                //
                // ⚠ 制約検証は**キャッシュ引きの前に毎回**行う（関数側と同じ・エラー意味論を保つ）。
                // ⚠ 同じ型引数の具体化は**同じクラス**（展開時に 1 回だけ作る）。`Value::Class` の等値
                //    （class_id 比較）も R3 の属性 IC もこれを前提にしている（D3）。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ **具体化は展開時に作る**（D36・タスク 2-16）。展開器が置いたクラスは、定義された
                //   時点でこのキャッシュに入る（`register_mono_instance`）。実行時には作らない。
                //   クラス変数の初期化もその定義で済んでいる。
                let Some(cls) = self.template_class_cache.get(&key).cloned() else {
                    return Err(Self::not_instantiated(&tmpl.name, type_args));
                };
                // ⚠ 引数の評価は**クラスが決まった後**（既存の順序を保つ。先に評価すると
                //   制約検証より前に副作用が起きる）。
                let evaled = call_args.into_evaled(self)?;
                self.instantiate_evaled(cls, evaled)
            }
            Value::TemplateGenFn(tmpl) => {
                // テンプレートジェネレータ関数: 展開時に作った具体化をジェネレータとして実行する。
                self.check_template_constraints(&tmpl.template_params, type_args)?;
                let key = (Rc::as_ptr(&tmpl) as usize, type_args.to_vec());
                // ⚠⚠ **具体化は展開時に作る**（D36・タスク 2-16・関数と同じ）。
                let Some(gen_fn) = self.template_gen_cache.get(&key).cloned() else {
                    return Err(Self::not_instantiated(&tmpl.name, type_args));
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

    /// 展開時に作られていない具体化を実行しようとしたときの文面（D36・タスク 2-16）。
    ///
    /// ⚠ ふつうは起きない（展開器が書かれた具体化をすべて作る）。起きるのは、デバッガで新しい
    ///   具体化を書いたとき・関数の中の `import` のように展開器が降りない場所のテンプレートだけ。
    fn not_instantiated(name: &str, type_args: &[String]) -> String {
        format!(
            "TemplateError: '{}' is not instantiated — template instantiations are made before the \
             program runs (in the REPL, when a block is run), and this one was not made there; the \
             debugger cannot make new instantiations",
            crate::template_subst::instance_name(name, type_args)
        )
    }

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


}

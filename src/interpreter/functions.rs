// functions.rs — 関数・ジェネレータ・オーバーロード実行
// (exec_fn_evaled / exec_fn / exec_generator / eval_call_args / bind_args /
//  dispatch_overload / dispatch_overload_evaled / overload_types_match / value_matches_ann)
//
// 関数・ジェネレータ関数の実行と、オーバーロード解決ロジックを提供する。
// 実行時には独立したスコープを構築し、関数完了後に呼び出し元のスコープを復元する。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{CallArg, Param};

use super::{
    Interpreter, Value, Var, FnValue, GeneratorFnValue, GeneratorState,
    ExecResult, StackFrame, DictData, InstanceData,
    RAISE_SENTINEL, GENERATOR_YIELDS,
};

impl Interpreter {
    /// 評価済み引数リストを用いて関数を実行する。
    ///
    /// 実行フロー:
    /// 1. `bind_args` で引数を仮引数にバインド
    /// 2. グローバルスコープ以外を一時退避し、新しいローカルスコープを構築
    /// 3. メソッド呼び出しの場合は `Self` を現在のクラスにバインド
    /// 4. 関数本体を実行
    /// 5. スコープを復元
    /// 6. 例外が伝播している場合はトレースバックフレームを追加
    ///
    /// - `fn_val`: 実行する関数定義
    /// - `evaled`: 評価済み引数リスト（位置引数は `None`、キーワード引数は `Some(name)`）
    /// - `self_val`: レシーバインスタンス（メソッド呼び出し時は `Some`、通常関数は `None`）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: `Ok(Value)` — `return` 値または `None`。`Err(message)` — ランタイムエラーまたは例外センチネル
    pub(super) fn exec_fn_evaled(
        &mut self,
        fn_val: Rc<FnValue>,
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
        fn_name: &str,
    ) -> Result<Value, String> {
        let (bindings, extra_kwargs) = if fn_val.is_python {
            Self::bind_args_relaxed(&fn_val.params, evaled, self_val.clone())?
        } else {
            (Self::bind_args(&fn_val.params, evaled, self_val.clone())?, vec![])
        };

        // グローバルスコープ（インデックス 0）以外を一時退避して関数専用スコープを構築する
        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();
        for (name, val, mutable) in bindings {
            self.declare_var(name, Var { value: val, mutable });
        }
        // Python 関数: 引数リストにない余分なキーワード引数を kwargs dict に注入する
        if fn_val.is_python && !extra_kwargs.is_empty() {
            let mut dict = DictData::new("str".to_string(), "Any".to_string());
            for (k, v) in extra_kwargs {
                dict.set(Value::Str(k), v);
            }
            self.declare_var(
                "kwargs".to_string(),
                Var { value: Value::Dict(Rc::new(RefCell::new(dict))), mutable: false },
            );
        }
        // メソッド実行時: `Self` をレシーバインスタンスのクラスにバインドする
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var("Self".to_string(), Var { value: Value::Class(class), mutable: false });
        }

        self.call_stack.push(fn_name.to_string());
        let result = self.exec_block(&fn_val.body);
        self.call_stack.pop();

        // スコープを復元する（グローバルのみ残してから退避分を追記）
        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // 例外が ExecResult::Raise として直接返ってきた場合: フレームを追加してセンチネルを返す
        if let Ok(ExecResult::Raise(mut raised)) = result {
            raised.frames.push(StackFrame {
                file: String::new(),
                line: 0,
                col: 0,
                fn_name: fn_name.to_string(),
                context: String::new(),
            });
            self.current_exception = Some(raised);
            return Err(RAISE_SENTINEL.to_string());
        }

        // 例外センチネルが Err として伝播してきた場合（ネストした関数からの raise）: フレームを追加する
        if let Err(ref e) = result {
            if e.as_str() == RAISE_SENTINEL {
                if let Some(ref mut raised) = self.current_exception {
                    raised.frames.push(StackFrame {
                        file: String::new(),
                        line: 0,
                        col: 0,
                        fn_name: fn_name.to_string(),
                        context: String::new(),
                    });
                }
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        match result? {
            ExecResult::Return(v) => Ok(v),
            ExecResult::Normal | ExecResult::BlockReturn(_) => Ok(Value::None),
            ExecResult::Break => Err("SyntaxError: 'break' outside loop".to_string()),
            ExecResult::Continue => Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Raise(_) => unreachable!("Raise already handled above"),
        }
    }

    /// 呼び出し引数式リストを評価してから関数を実行する。`exec_fn_evaled` の呼び出しラッパー。
    ///
    /// - `fn_val`: 実行する関数定義
    /// - `call_args`: 評価前の呼び出し引数リスト（AST の `CallArg`）
    /// - `self_val`: レシーバインスタンス（メソッド用）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: 関数の実行結果（`exec_fn_evaled` と同じ）
    pub(super) fn exec_fn(
        &mut self,
        fn_val: Rc<FnValue>,
        call_args: &[CallArg],
        self_val: Option<Value>,
        fn_name: &str,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        self.exec_fn_evaled(fn_val, &evaled, self_val, fn_name)
    }

    /// ジェネレータ関数の本体を一括実行し、すべての `yield` 値を収集して `Value::Generator` を返す。
    ///
    /// スレッドローカル `GENERATOR_YIELDS` を `Some(Vec::new())` にセットして yield 収集を有効化し、
    /// 本体実行後に収集した値リストを取り出して `GeneratorState` を構築する。
    ///
    /// - `gen_fn`: 実行するジェネレータ関数定義
    /// - `call_args`: 呼び出し引数リスト（AST の `CallArg`）
    /// - `self_val`: レシーバインスタンス（ジェネレータメソッド用; `None` はスタンドアロン）
    ///
    /// 戻り値: `Ok(Value::Generator)` — 収集済みの yield 値を保持するジェネレータ。
    ///         `Err(message)` — ランタイムエラーまたは例外センチネル
    pub(super) fn exec_generator(&mut self, gen_fn: Rc<GeneratorFnValue>, call_args: &[CallArg], self_val: Option<Value>) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        let bindings = Self::bind_args(&gen_fn.params, &evaled, self_val.clone())?;

        // yield 収集を有効化する（スレッドローカルに収集先を設定）
        GENERATOR_YIELDS.with(|y| {
            *y.borrow_mut() = Some(Vec::new());
        });

        // exec_fn_evaled と同様にグローバルスコープ以外を退避して独立したスコープで実行する
        let outer_scopes: Vec<_> = self.scopes.drain(1..).collect();
        self.push_scope();
        for (name, val, mutable) in bindings {
            self.declare_var(name, Var { value: val, mutable });
        }
        // ジェネレータメソッド実行時: `Self` をレシーバインスタンスのクラスにバインドする
        if let Some(Value::Instance(inst_rc)) = &self_val {
            let class = inst_rc.borrow().class.clone();
            self.declare_var("Self".to_string(), Var { value: Value::Class(class), mutable: false });
        }
        let exec_result = self.exec_block(&gen_fn.body);
        self.scopes.truncate(1);
        self.scopes.extend(outer_scopes);

        // エラー時も含めて必ずスレッドローカルをクリーンアップして yield 値を回収する
        let yields = GENERATOR_YIELDS.with(|y| y.borrow_mut().take().unwrap_or_default());

        match exec_result? {
            ExecResult::Normal | ExecResult::BlockReturn(_) => {}
            ExecResult::Break    => return Err("SyntaxError: 'break' outside loop".to_string()),
            ExecResult::Continue => return Err("SyntaxError: 'continue' outside loop".to_string()),
            ExecResult::Return(_) => {} // パーサーが gen 内の return を禁止しているためここには到達しない
            ExecResult::Raise(raised) => {
                self.current_exception = Some(raised);
                return Err(RAISE_SENTINEL.to_string());
            }
        }

        Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState { values: yields, index: 0 }))))
    }

    /// 呼び出し引数リスト（AST の `CallArg`）を評価して `(name, value)` ペアのリストを返す。
    ///
    /// - 位置引数: `(None, value)` として格納
    /// - キーワード引数: `(Some(name), value)` として格納
    ///
    /// - `call_args`: 評価前の呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Vec<(Option<String>, Value)>)` — 評価済み引数リスト。`Err` — 評価エラー
    pub(super) fn eval_call_args(&mut self, call_args: &[CallArg]) -> Result<Vec<(Option<String>, Value)>, String> {
        let mut result = Vec::new();
        for arg in call_args {
            match arg {
                CallArg::Positional(e) => result.push((None, self.eval(e)?)),
                CallArg::Keyword { name, value } => result.push((Some(name.clone()), self.eval(value)?)),
            }
        }
        Ok(result)
    }

    /// 評価済み引数リストを仮引数リストにバインドして `(name, value, mutable)` トリプルのリストを返す。
    ///
    /// バインドルール:
    /// - `self_val` が `Some` かつ先頭パラメータが `self` の場合: `self` を先にバインド
    /// - 位置引数: 順番にパラメータスロットに割り当てる
    /// - キーワード引数: パラメータ名で検索してスロットに割り当てる
    /// - 引数数が一致しない場合や重複キーワードは `TypeError` を返す
    ///
    /// - `params`: 仮引数リスト
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（`None` の場合は通常の引数バインド）
    ///
    /// 戻り値: `Ok(Vec<(name, value, mutable)>)` — バインド済みリスト。`Err` — 引数エラー
    pub(super) fn bind_args(
        params: &[Param],
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
    ) -> Result<Vec<(String, Value, bool)>, String> {
        let mut result = Vec::new();

        // self_val が Some かつ先頭パラメータが "self" なら先にバインドして残りのパラメータを取得する
        let params_to_bind = if let (Some(sv), Some(p)) = (&self_val, params.first()) {
            if p.name == "self" {
                // let self（不変レシーバ）はディープコピーして元オブジェクトの変更を防ぐ
                let self_to_bind = if p.mutable { sv.clone() } else { Self::deep_copy_value(sv.clone()) };
                result.push(("self".to_string(), self_to_bind, p.mutable));
                &params[1..]
            } else {
                params
            }
        } else {
            params
        };

        if evaled.len() != params_to_bind.len() {
            return Err(format!(
                "TypeError: function takes {} argument(s), got {}",
                params_to_bind.len(),
                evaled.len()
            ));
        }

        // パラメータスロットを用意して位置引数・キーワード引数を割り当てる
        let mut slots: Vec<Option<Value>> = vec![None; params_to_bind.len()];
        let mut positional_idx = 0usize;

        for (key, val) in evaled {
            match key {
                None => {
                    // 位置引数: 次のスロットに順番に割り当てる
                    slots[positional_idx] = Some(val.clone());
                    positional_idx += 1;
                }
                Some(name) => {
                    // キーワード引数: パラメータ名でスロットを検索して割り当てる
                    let pos = params_to_bind.iter().position(|p| p.name == *name)
                        .ok_or_else(|| format!("TypeError: unexpected keyword argument '{name}'"))?;
                    if slots[pos].is_some() {
                        return Err(format!("TypeError: argument '{name}' given twice"));
                    }
                    slots[pos] = Some(val.clone());
                }
            }
        }

        // 未割り当てスロットがあれば missing argument エラーを返す
        // let パラメータ（not mut）には参照型をディープコピーして渡す
        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => {
                    let param = &params_to_bind[i];
                    let value = if param.mutable { v } else { Self::deep_copy_value(v) };
                    result.push((param.name.clone(), value, param.mutable));
                }
                None => return Err(format!("TypeError: missing argument '{}'", params_to_bind[i].name)),
            }
        }

        Ok(result)
    }

    /// Python 関数用の引数バインド。`bind_args` と同じだが、パラメータリストにないキーワード引数を
    /// エラーにせず `extra_kwargs` として返す。引数個数の検査は位置引数のみで行う。
    ///
    /// 戻り値: `Ok((bindings, extra_kwargs))` — `bindings` は通常通り、`extra_kwargs` は余分な kwarg の (名前, 値) リスト
    pub(super) fn bind_args_relaxed(
        params: &[Param],
        evaled: &[(Option<String>, Value)],
        self_val: Option<Value>,
    ) -> Result<(Vec<(String, Value, bool)>, Vec<(String, Value)>), String> {
        let mut result = Vec::new();
        let mut extra_kwargs = Vec::new();

        let params_to_bind = if let (Some(sv), Some(p)) = (&self_val, params.first()) {
            if p.name == "self" {
                result.push(("self".to_string(), sv.clone(), p.mutable));
                &params[1..]
            } else {
                params
            }
        } else {
            params
        };

        let mut slots: Vec<Option<Value>> = vec![None; params_to_bind.len()];
        let mut positional_idx = 0usize;

        for (key, val) in evaled {
            match key {
                None => {
                    if positional_idx >= params_to_bind.len() {
                        return Err(format!(
                            "TypeError: function takes {} positional argument(s), got too many",
                            params_to_bind.len()
                        ));
                    }
                    slots[positional_idx] = Some(val.clone());
                    positional_idx += 1;
                }
                Some(name) => {
                    match params_to_bind.iter().position(|p| p.name == *name) {
                        Some(pos) => {
                            if slots[pos].is_some() {
                                return Err(format!("TypeError: argument '{name}' given twice"));
                            }
                            slots[pos] = Some(val.clone());
                        }
                        None => {
                            extra_kwargs.push((name.clone(), val.clone()));
                        }
                    }
                }
            }
        }

        for (i, slot) in slots.into_iter().enumerate() {
            match slot {
                Some(v) => result.push((params_to_bind[i].name.clone(), v, params_to_bind[i].mutable)),
                None => return Err(format!("TypeError: missing argument '{}'", params_to_bind[i].name)),
            }
        }

        Ok((result, extra_kwargs))
    }

    // --- オーバーロード解決 ---

    /// 呼び出し引数を評価してからオーバーロード候補を解決して実行する。
    /// 引数の評価は一度だけ行い、`dispatch_overload_evaled` に委譲する。
    ///
    /// - `candidates`: オーバーロード候補の関数リスト
    /// - `args`: 呼び出し引数リスト（評価前）
    /// - `self_val`: レシーバインスタンス（メソッド用）
    ///
    /// 戻り値: 選択されたオーバーロードの実行結果
    pub(super) fn dispatch_overload(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        args: &[CallArg],
        self_val: Option<Value>,
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(args)?;
        self.dispatch_overload_evaled(candidates, evaled, self_val, "<overloaded>")
    }

    /// 評価済み引数リストを用いてオーバーロード候補から適合する関数を選択して実行する。
    ///
    /// 解決アルゴリズム:
    /// 1. `self` を除いた有効引数数でフィルタリング
    /// 2. 引数数が一致する候補が1つなら即実行
    /// 3. 複数一致する場合は型アノテーションと引数型を照合（`overload_types_match`）
    /// 4. 型一致候補が見つからない場合は引数数一致の先頭候補にフォールバック
    ///
    /// - `candidates`: オーバーロード候補リスト
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（メソッド用）
    /// - `fn_name`: トレースバックフレーム用の関数名
    ///
    /// 戻り値: 選択されたオーバーロードの実行結果。`Err` — 引数数不一致など
    pub(super) fn dispatch_overload_evaled(
        &mut self,
        candidates: Vec<Rc<FnValue>>,
        evaled: Vec<(Option<String>, Value)>,
        self_val: Option<Value>,
        fn_name: &str,
    ) -> Result<Value, String> {
        let call_count = evaled.len();
        let has_self = self_val.is_some();

        // `self` パラメータを除いた有効引数数を計算するクロージャ
        let effective_param_count = |f: &FnValue| -> usize {
            let self_offset = if has_self && f.params.first().map(|p| p.name == "self").unwrap_or(false) { 1 } else { 0 };
            f.params.len() - self_offset
        };

        // 引数数が一致する候補のみに絞り込む
        let count_matching: Vec<Rc<FnValue>> = candidates.iter()
            .filter(|f| effective_param_count(f) == call_count)
            .cloned()
            .collect();

        if count_matching.is_empty() {
            let available: Vec<String> = candidates.iter()
                .map(|f| effective_param_count(f).to_string())
                .collect();
            return Err(format!(
                "TypeError: no overload takes {} argument(s) (overloads take: {})",
                call_count, available.join(", ")
            ));
        }

        if count_matching.len() == 1 {
            return self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name);
        }

        // 引数数が複数一致: 型アノテーションと引数型を照合して候補を絞り込む
        for candidate in &count_matching {
            if Self::overload_types_match(candidate, &evaled, &self_val) {
                return self.exec_fn_evaled(candidate.clone(), &evaled, self_val.clone(), fn_name);
            }
        }

        // 型一致候補なし: 引数数一致の先頭候補にフォールバック
        self.exec_fn_evaled(count_matching[0].clone(), &evaled, self_val, fn_name)
    }

    /// 関数のすべてのアノテーション付きパラメータが対応する引数値と型一致するか判定する。
    ///
    /// アノテーションのないパラメータは型チェックをスキップする。
    /// 型一致の判定には `value_matches_ann` を使用する。
    ///
    /// - `fn_val`: 型チェック対象の関数定義
    /// - `evaled`: 評価済み引数リスト
    /// - `self_val`: レシーバインスタンス（`Some` の場合は `self` パラメータをスキップ）
    ///
    /// 戻り値: `true` — すべてのアノテーション付きパラメータが一致
    pub(super) fn overload_types_match(
        fn_val: &FnValue,
        evaled: &[(Option<String>, Value)],
        self_val: &Option<Value>,
    ) -> bool {
        // self_val がある場合は `self` パラメータをスキップして残りを対象にする
        let params = if self_val.is_some() && fn_val.params.first().map(|p| p.name == "self").unwrap_or(false) {
            &fn_val.params[1..]
        } else {
            &fn_val.params[..]
        };

        // 各引数をパラメータスロットに割り当てる（bind_args と同様のロジック）
        let mut slots: Vec<Option<&Value>> = vec![None; params.len()];
        let mut positional_idx = 0usize;

        for (key, val) in evaled {
            match key {
                None => {
                    if positional_idx >= params.len() { return false; }
                    slots[positional_idx] = Some(val);
                    positional_idx += 1;
                }
                Some(name) => {
                    if let Some(pos) = params.iter().position(|p| p.name == *name) {
                        slots[pos] = Some(val);
                    } else {
                        return false;
                    }
                }
            }
        }

        // アノテーション付きパラメータについてのみ型一致を確認する
        for (i, slot) in slots.iter().enumerate() {
            if let (Some(val), Some(ann)) = (slot, &params[i].type_ann) {
                if !Self::value_matches_ann(val, ann) {
                    return false;
                }
            }
        }
        true
    }

    /// 値が型アノテーション名と一致するかを判定する（オーバーロード解決用）。
    ///
    /// - `val`: チェック対象の値
    /// - `ann`: パラメータの型アノテーション名
    ///
    /// 戻り値: `true` — 型が一致する
    pub(super) fn value_matches_ann(val: &Value, ann: &str) -> bool {
        // `tuple` アノテーションは任意の Tuple 値に一致する（要素数・型は問わない）
        if ann == "tuple" && matches!(val, Value::Tuple(_)) { return true; }
        matches!(
            (ann, val),
            ("int",   Value::Int(_))
            | ("float", Value::Float(_))
            | ("str",   Value::Str(_))
            | ("bool",  Value::Bool(_))
            | ("None",  Value::None)
            | ("list",  Value::List(_))
            | ("type",  Value::Type(_))
            | ("type",  Value::Class(_))
            | ("Self",  Value::Instance(_))
        )
    }

    /// 参照型の値を再帰的にディープコピーして返す。
    ///
    /// `let` パラメータへのバインド時に呼ばれ、元の可変変数（`mut`）が
    /// 関数内部から変更されることを防ぐ。
    ///
    /// 変換規則:
    /// - `Instance`: フィールドを再帰コピーして新しい `InstanceData` を生成する
    /// - `Dict`: キー・値を再帰コピーして新しい `DictData` を生成する
    /// - `List`: 各要素を再帰コピーする
    /// - その他: プリミティブ・不変型はそのまま返す（Rust の clone でコピー済み）
    fn deep_copy_value(val: Value) -> Value {
        match val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let new_fields = inst.fields.iter()
                    .map(|(k, (v, m))| (k.clone(), (Self::deep_copy_value(v.clone()), *m)))
                    .collect();
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    class: inst.class.clone(),
                    fields: new_fields,
                    immutable: inst.immutable,
                })))
            }
            Value::Dict(d) => {
                let d_ref = d.borrow();
                let mut new_dict = DictData::new(d_ref.key_type.clone(), d_ref.item_type.clone());
                for (k, v) in d_ref.keys.iter().zip(d_ref.items.iter()) {
                    new_dict.set(Self::deep_copy_value(k.clone()), Self::deep_copy_value(v.clone()));
                }
                Value::Dict(Rc::new(RefCell::new(new_dict)))
            }
            Value::List(items) => Value::List(items.into_iter().map(Self::deep_copy_value).collect()),
            // Tuple は Rc<TupleData> だが TupleData は不変なので共有で問題なし
            // プリミティブ・関数・クラス等はそのまま返す
            other => other,
        }
    }
}

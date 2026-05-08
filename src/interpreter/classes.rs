// classes.rs — クラス・インスタンス管理
// (instantiate / eval_method_call / lookup_method_in_class / lookup_class_var / freeze_instance)
//
// クラスのインスタンス化、メソッド呼び出し、継承チェーンを辿るメソッド・クラス変数の検索を提供する。
// List / Str / Dict / Generator などの組み込み型のメソッドディスパッチもここで行う。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::CallArg;

use super::{Interpreter, Value, ClassValue, FnValue, InstanceData, GeneratorState};

impl Interpreter {
    // --- インスタンスの凍結 ---

    /// インスタンスを不変化する: `immutable = true` にセットし、すべての `mut` フィールドを不変にする。
    ///
    /// `let` バインドされたインスタンスに適用される。以降は `mut self` メソッド呼び出しが禁止される。
    ///
    /// - `inst_rc`: 不変化するインスタンスへの共有参照
    pub(super) fn freeze_instance(inst_rc: &Rc<RefCell<InstanceData>>) {
        let mut inst = inst_rc.borrow_mut();
        inst.immutable = true;
        // すべてのフィールドを不変に変更する
        for (_, mutable) in inst.fields.values_mut() {
            *mutable = false;
        }
    }

    // --- クラスのインスタンス化 ---

    /// クラスを引数付きでインスタンス化して `Value::Instance` を返す。
    ///
    /// 処理フロー:
    /// 1. `field_defaults` からデフォルトフィールドを初期化
    /// 2. `InstanceData` を構築して `Rc<RefCell>` に包む
    /// 3. `__init__` メソッドを呼び出す（オーバーロードがある場合は `dispatch_overload`）
    ///
    /// - `class`: インスタンス化するクラス定義
    /// - `call_args`: コンストラクタ引数リスト（AST の `CallArg`）
    ///
    /// 戻り値: `Ok(Value::Instance)` — 初期化済みインスタンス。`Err` — コンストラクタ実行エラー
    pub(super) fn instantiate(&mut self, class: Rc<ClassValue>, call_args: &[CallArg]) -> Result<Value, String> {
        // デフォルト値付きフィールドをインスタンスに事前設定する
        let mut fields = HashMap::new();
        for (name, default_val, mutable) in &class.field_defaults {
            fields.insert(name.clone(), (default_val.clone(), *mutable));
        }
        let inst_rc = Rc::new(RefCell::new(InstanceData { class: class.clone(), fields, immutable: false }));
        let inst_val = Value::Instance(inst_rc);

        // `__init__` を呼び出す（定義がない場合はスキップ）
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn(init_overloads[0].clone(), call_args, Some(inst_val.clone()), "__init__")?;
            } else {
                self.dispatch_overload(init_overloads, call_args, Some(inst_val.clone()))?;
            }
        }

        Ok(inst_val)
    }

    /// オブジェクトのメソッドを呼び出して結果を返す。
    ///
    /// 各値型に対してディスパッチを行う:
    /// - `List`: `__iter__()` のみ対応（組み込みジェネレータを返す）
    /// - `Str`: `__iter__()` のみ対応（文字ごとのジェネレータを返す）
    /// - `Instance`: `gen_methods`（ジェネレータメソッド）を優先し、次に通常メソッドを検索
    ///   - 不変インスタンスは `mut self` メソッドを呼べない
    ///   - オーバーロードがある場合は `dispatch_overload` で解決する
    /// - `Dict`: `key()` / `item()` のみ対応
    /// - `Generator`: `next()` のみ対応（枯渇時は `EndOfIteration` エラー）
    ///
    /// - `obj`: メソッドを呼び出す対象の値
    /// - `method_name`: 呼び出すメソッド名
    /// - `args`: 呼び出し引数リスト
    ///
    /// 戻り値: `Ok(Value)` — メソッドの返り値。`Err(message)` — AttributeError 等
    pub(super) fn eval_method_call(
        &mut self,
        obj: Value,
        method_name: &str,
        args: &[CallArg],
    ) -> Result<Value, String> {
        match &obj {
            Value::List(items) => {
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: list.__iter__() takes no arguments".to_string());
                    }
                    // リスト全要素をジェネレータにラップして返す
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: items.clone(),
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'list' object has no method '{method_name}'"))
            }
            Value::Str(s) => {
                if method_name == "__iter__" {
                    if !args.is_empty() {
                        return Err("TypeError: str.__iter__() takes no arguments".to_string());
                    }
                    // 文字列を1文字ずつ Value::Str に変換してジェネレータにラップする
                    let chars: Vec<Value> = s.chars().map(|c| Value::Str(c.to_string())).collect();
                    return Ok(Value::Generator(Rc::new(RefCell::new(GeneratorState {
                        values: chars,
                        index: 0,
                    }))));
                }
                Err(format!("AttributeError: 'str' object has no method '{method_name}'"))
            }
            Value::Instance(inst_rc) => {
                let class = inst_rc.borrow().class.clone();
                let inst_immutable = inst_rc.borrow().immutable;

                // gen_methods（`gen` キーワードで定義されたメソッド、例: `__iter__`）を優先的にチェック
                if let Some(gen_fn) = class.gen_methods.get(method_name).cloned() {
                    return self.exec_generator(gen_fn, args, Some(obj.clone()));
                }

                let overloads = self.lookup_method_in_class(&class, method_name)
                    .ok_or_else(|| format!("AttributeError: '{}' has no method '{method_name}'", class.name))?;

                // 不変インスタンスは `mut self` を要求するオーバーロードを除外する
                let callable: Vec<Rc<FnValue>> = if inst_immutable {
                    overloads.iter().filter(|f| {
                        f.params.first().map(|p| p.name != "self" || !p.mutable).unwrap_or(true)
                    }).cloned().collect()
                } else {
                    overloads
                };

                if callable.is_empty() {
                    return Err(format!(
                        "TypeError: cannot call mutable method '{method_name}' on immutable instance of '{}'",
                        class.name
                    ));
                }

                if callable.len() == 1 {
                    self.exec_fn(callable[0].clone(), args, Some(obj.clone()), method_name)
                } else {
                    self.dispatch_overload(callable, args, Some(obj.clone()))
                }
            }
            Value::Dict(d) => {
                match method_name {
                    // `d.key()` — キーのリストを返す
                    "key" => {
                        if !args.is_empty() {
                            return Err("TypeError: dict.key() takes no arguments".to_string());
                        }
                        Ok(Value::List(d.borrow().all_keys()))
                    }
                    // `d.item()` — 値のリストを返す
                    "item" => {
                        if !args.is_empty() {
                            return Err("TypeError: dict.item() takes no arguments".to_string());
                        }
                        Ok(Value::List(d.borrow().all_items()))
                    }
                    _ => Err(format!("AttributeError: 'dict' object has no method '{method_name}'")),
                }
            }
            Value::Generator(state) => {
                if method_name != "next" {
                    return Err(format!(
                        "AttributeError: Generator object has no method '{method_name}'"
                    ));
                }
                if !args.is_empty() {
                    return Err("TypeError: Generator.next() takes no arguments".to_string());
                }
                let mut s = state.borrow_mut();
                if s.index < s.values.len() {
                    // 次の yield 値を返してインデックスを進める
                    let val = s.values[s.index].clone();
                    s.index += 1;
                    Ok(val)
                } else {
                    // ジェネレータが枯渇した: for ループはこのエラーでループを終了する
                    Err("EndOfIteration: generator is exhausted".to_string())
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no method '{method_name}'",
                self.type_name(&obj)
            )),
        }
    }

    /// メソッドをクラスから検索する。クラス本体の `methods` マップのみを参照する。
    ///
    /// 注意: クラス間継承は無効化されており、trait ベースの継承のみパース時にサポートされる。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `method_name`: 検索するメソッド名
    ///
    /// 戻り値: `Some(Vec<Rc<FnValue>>)` — オーバーロード候補リスト。`None` — 見つからない
    pub(super) fn lookup_method_in_class(&self, class: &Rc<ClassValue>, method_name: &str) -> Option<Vec<Rc<FnValue>>> {
        if let Some(overloads) = class.methods.get(method_name) {
            return Some(overloads.clone());
        }
        // クラス間継承は無効。trait ベースの継承はパース時にのみサポートされる。
        None
    }

    /// `const` クラス変数をクラスの `class_vars` から検索する。
    ///
    /// 現在はクラス変数の継承（基底クラスへの遡及検索）は未実装。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `name`: クラス変数名
    ///
    /// 戻り値: `Some(Value)` — クラス変数の値。`None` — 見つからない
    pub(super) fn lookup_class_var(class: &Rc<ClassValue>, name: &str) -> Option<Value> {
        class.class_vars.get(name).cloned()
        // 注: 基底クラスへの遡及検索にはスコープへのアクセスが必要なため、現在は未実装
    }
}

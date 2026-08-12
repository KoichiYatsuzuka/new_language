// classes/instantiate.rs — クラスのインスタンス化: instantiate / instantiate_evaled。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::CallArg,
    crate::interpreter::{
        ClassValue, InstanceData,
        Interpreter, Value,
    },
};

impl Interpreter {
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
    pub(crate) fn instantiate(
        &mut self,
        class: Rc<ClassValue>,
        call_args: &[CallArg],
    ) -> Result<Value, String> {
        let evaled = self.eval_call_args(call_args)?;
        self.instantiate_evaled(class, evaled)
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
    /// 評価済み引数リストでクラスをインスタンス化する（デコレータ適用などに使用）。
    pub(crate) fn instantiate_evaled(
        &mut self,
        class: Rc<ClassValue>,
        evaled: Vec<(Option<String>, Value, bool)>,
    ) -> Result<Value, String> {
        let mut inst = InstanceData::new_empty(class.clone(), 0);
        for (name, default_val, mutable) in &class.field_defaults {
            if let Some(&idx) = class.field_index.get(name.as_str()) {
                inst.store_field(idx, default_val.clone(), *mutable);
            }
        }
        let inst_rc = Rc::new(RefCell::new(inst));
        let inst_val = Value::Instance(inst_rc);
        let class_name = class.name.clone();

        // ── cs-dll / cs-proc ブリッジ経由のコンストラクタ（#22-c で移植）──
        // 以前は `instantiate`（CallArg 版）にしか無く、`instantiate_evaled` は
        // ツリーウォークの `Class` アームからしか呼ばれていなかったため露見していなかった。
        // 22-c で `Namespace` アームを C 軸へ委譲した際に `cs_interop_test.ar` が
        // `TypeError: function takes 0 argument(s), got 1` で落ちて発覚した
        // （ブリッジ分岐が無く、引数無しの Arrow 側 stub `__init__` へ流れていた）。
        if let Some(Value::Str(bp)) = class.class_vars.get("__cs_bridge_path__") {
            let bp_path = std::path::PathBuf::from(&**bp);
            let arg_vals: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
            if let Some(bridge) = crate::interpreter::cs_dll_runtime::get_bridge(&bp_path) {
                let handle =
                    crate::interpreter::cs_dll_runtime::call_constructor(&bridge, &class_name, &arg_vals)
                        .map_err(|e| format!("CsDll: constructor for '{class_name}' failed: {e}"))?;
                return Ok(Value::CsObject(Rc::new(crate::interpreter::value::CsObjectData {
                    class_name: class_name.clone(),
                    handle,
                    bridge_path: bp_path,
                    class: class.clone(),
                    is_proc: false,
                })));
            }
        }
        if let Some(Value::Str(pp)) = class.class_vars.get("__cs_proc_path__") {
            let pp_path = std::path::PathBuf::from(&**pp);
            let arg_vals: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
            let handle =
                crate::interpreter::cs_proc_runtime::call_constructor(&pp_path, &class_name, &arg_vals)
                    .map_err(|e| format!("CsProc: constructor for '{class_name}' failed: {e}"))?;
            return Ok(Value::CsObject(Rc::new(crate::interpreter::value::CsObjectData {
                class_name: class_name.clone(),
                handle,
                bridge_path: pp_path,
                class: class.clone(),
                is_proc: true,
            })));
        }

        // Native __init__ dispatch
        if crate::interpreter::native_api::lookup_native_method_ptr(&class_name, "__init__").is_some() {
            let arg_vals: Vec<Value> = evaled.into_iter().map(|(_, v, _)| v).collect();
            if let Some(result) = crate::interpreter::native_api::try_dispatch_native_method(
                self, inst_val.clone(), "__init__", arg_vals,
            ) {
                result?;
            }
            return Ok(inst_val);
        }
        if let Some(init_overloads) = self.lookup_method_in_class(&class, "__init__") {
            if init_overloads.len() == 1 {
                self.exec_fn_evaled(
                    init_overloads[0].clone(),
                    &evaled,
                    Some(inst_val.clone()),
                    "__init__",
                    None,
                )?;
            } else {
                self.dispatch_overload_evaled(
                    init_overloads,
                    evaled,
                    Some(inst_val.clone()),
                    "__init__",
                    None,
                )?;
            }
        }
        Ok(inst_val)
    }

}

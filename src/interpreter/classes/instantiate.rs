// classes/instantiate.rs — クラスのインスタンス化: instantiate / instantiate_evaled。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::interpreter::{
        ClassValue, InstanceData,
        Interpreter, Value,
    },
};

impl Interpreter {
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

        // ── cs-dll / cs-proc ブリッジ経由のコンストラクタ（#22-c で移植）──
        // 以前は `instantiate`（CallArg 版）にしか無く、`instantiate_evaled` は
        // ツリーウォークの `Class` アームからしか呼ばれていなかったため露見していなかった。
        // 22-c で `Namespace` アームを C 軸へ委譲した際に `cs_interop_test.ar` が
        // `TypeError: function takes 0 argument(s), got 1` で落ちて発覚した
        // （ブリッジ分岐が無く、引数無しの Arrow 側 stub `__init__` へ流れていた）。
        // C# のブリッジのクラスはクラス変数に経路を持つ。クラス変数の無いクラス（大半）は引かない（10-1）
        let bridge_var = |key: &str| (!class.class_vars.is_empty()).then(|| class.class_vars.get(key)).flatten();
        if let Some(Value::Str(bp)) = bridge_var("__cs_bridge_path__") {
            let class_name = class.name.clone();
            let bp_path = std::path::PathBuf::from(&**bp);
            let arg_vals: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
            // ⚠ ブリッジが無ければエラー（6-2。以前は Arrow のスタブの `__init__` へ落ちて、C# のオブジェクトでない
            //   インスタンスを黙って作った）
            let Some(bridge) = crate::interpreter::cs_dll_runtime::get_bridge(&bp_path) else {
                return Err(format!("CsDll: bridge DLL not loaded for '{class_name}'"));
            };
            {
                let handle = crate::interpreter::cs_values::ctor_types(&class, arg_vals.len())
                    .and_then(|types| {
                        crate::interpreter::cs_dll_runtime::call_constructor(&bridge, &class_name, &arg_vals, &types)
                    })
                    .map_err(|e| crate::interpreter::cs_values::tag_error(
                        &format!("CsDll: constructor for '{class_name}' failed"), e))?;
                return Ok(Value::CsObject(Rc::new(crate::interpreter::value::CsObjectData {
                    class_name: class_name.clone(),
                    handle,
                    bridge_path: bp_path,
                    class: class.clone(),
                    is_proc: false,
                })));
            }
        }
        if let Some(Value::Str(pp)) = bridge_var("__cs_proc_path__") {
            let class_name = class.name.clone();
            let pp_path = std::path::PathBuf::from(&**pp);
            let arg_vals: Vec<Value> = evaled.iter().map(|(_, v, _)| v.clone()).collect();
            let handle =
                crate::interpreter::cs_proc_runtime::call_constructor(&pp_path, &class_name, &arg_vals)
                    .map_err(|e| crate::interpreter::cs_values::tag_error(
                        &format!("CsProc: constructor for '{class_name}' failed"), e))?;
            return Ok(Value::CsObject(Rc::new(crate::interpreter::value::CsObjectData {
                class_name: class_name.clone(),
                handle,
                bridge_path: pp_path,
                class: class.clone(),
                is_proc: true,
            })));
        }

        // パーサが合成した `__init__`（本体が `self.f = f` だけ）は呼ばずに、VM と同じ規則で格納する（10-3・D-11・U-4）
        if let Some(plan) = self.auto_init_plan(&class) {
            if let Some(r) = self.store_auto_init(&plan, &inst_val, &evaled) {
                return r.map(|()| inst_val);
            }
        }

        // Native __init__ dispatch
        // 引数は宣言どおりに束縛し、`let` 仮引数は VM と同じ規則で複製する（6-3）。受け手は作ったばかりの値。
        // 表に無ければ `None`（表を引くのは 1 回だけ）。
        if let Some(result) = crate::interpreter::native_api::try_dispatch_native_method_evaled(
            self, inst_val.clone(), "__init__", &evaled, true,
        ) {
            result?;
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

/// パーサが合成した `__init__` の直接格納の計画の 1 項目（native_object_access_plan.md 10-3）: 仮引数（`self` を除く）と、
/// 格納するフィールドの番号・可変性（`attr_assign_ic` の外れた道と同じく名前の表 `field_mutability` から取る）。
pub(crate) struct AutoInitSlot {
    param: crate::ast::Param,
    idx: usize,
    is_mutable: bool,
}

impl Interpreter {
    /// `class` の `__init__` がパーサの合成したもの（`classes.rs` の `self.f = f` の並び・位置を持たない）で、どのフィールドも
    /// ふつうのインスタンスのフィールドなら、その直接格納の計画（クラスの番号ごとに 1 回だけ作る）。
    /// ⚠ 合成したものに限る（U-4 案 1）: 利用者が書いた同じ形の `__init__` は今までどおり呼ぶ（トレースバック・デバッガに出る）。
    ///   目印は文の位置: 合成した文はすべて `Span::unknown()`（行 0）で、パーサが読んだ文は必ず位置を持つ。
    fn auto_init_plan(&mut self, class: &Rc<ClassValue>) -> Option<Rc<[AutoInitSlot]>> {
        if let Some(p) = self.auto_inits.get(&class.class_id) {
            return p.clone();
        }
        let plan = Self::build_auto_init_plan(class);
        self.auto_inits.insert(class.class_id, plan.clone());
        plan
    }

    fn build_auto_init_plan(class: &Rc<ClassValue>) -> Option<Rc<[AutoInitSlot]>> {
        use crate::ast::{Expr, Stmt};
        let ov = class.methods.get("__init__")?;
        let [f] = ov.as_slice() else { return None };
        if f.is_python || !f.params.first().is_some_and(|p| p.name == "self") {
            return None;
        }
        // `import[rs]` の構造体のフィールドは setter を通す（`attr_assign_ic` の外れた道）
        if class.field_index.contains_key("__rs_handle__") {
            return None;
        }
        let params = &f.params[1..];
        if params.iter().any(|p| p.variadic || p.default.is_some()) || f.body.len() != params.len() {
            return None;
        }
        let mut slots = Vec::with_capacity(params.len());
        for (st, p) in f.body.iter().zip(params) {
            let Stmt::AttrAssign { target: Expr::Attr { object, attr, .. }, value: Expr::Ident { name, .. }, span } = st else {
                return None;
            };
            let self_obj = matches!(object.as_ref(), Expr::Ident { name: s, .. } if s == "self");
            if !self_obj || span.line != 0 || attr != &p.name || name != &p.name {
                return None;
            }
            // `static mut`・クラス変数への代入は格納ではない（外れた道が扱う）
            if class.static_vars.contains_key(attr) || Self::lookup_class_var(class, attr).is_some() {
                return None;
            }
            let idx = *class.field_index.get(attr.as_str())?;
            let is_mutable = class.field_mutability.get(attr.as_str()).copied().unwrap_or(true);
            slots.push(AutoInitSlot { param: p.clone(), idx, is_mutable });
        }
        Some(slots.into())
    }

    /// 合成した `__init__` を呼ぶ代わりに、VM と同じ規則で引数をフィールドへ格納する。`None` は今の道（`__init__` を
    /// 呼ぶ）へ回す: 名前つき引数・個数の違い・`__cast__` が起きうる引数（`may_cast_on_bind`）。
    /// - 束縛は `try_fast_bind` と同じ `bind_param_value`（`let` で受ける `mut` の値の複製・`float` への昇格）
    /// - 格納は VM の `self.f = f` と同じ（`attr_assign_ic`: 深いコピー＋`store_field_checked`）。合成した `__init__` は
    ///   自分のクラスの中で走るのでアクセスの検査は常に通る
    /// - ⚠ 束縛で複製した値も格納でもう一度深いコピーをする（VM と同じ手順）。2 度目を省くと、束縛の複製
    ///   （`deep_copy_unfrozen`）でできた値を書き換えた後の深いコピーが重くなった（調査のベンチの k1 の後の k2 が 0.73x）。
    ///   省いても生成は 1.3% しか速くならない
    fn store_auto_init(
        &mut self,
        plan: &[AutoInitSlot],
        inst_val: &Value,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Option<Result<(), String>> {
        if evaled.len() != plan.len() || evaled.iter().any(|(k, _, _)| k.is_some()) {
            return None;
        }
        if plan.iter().zip(evaled).any(|(s, (_, v, _))| Self::may_cast_on_bind(&s.param, v)) {
            return None;
        }
        let Value::Instance(rc) = inst_val else { return None };
        Some((|| {
            for (s, (_, v, arg_mut)) in plan.iter().zip(evaled) {
                let v = self.bind_param_value(&s.param, v.clone(), *arg_mut)?;
                let v = Self::deep_copy_value(v);
                let mut inst = rc.borrow_mut();
                Self::store_field_checked(&mut inst, s.idx, &s.param.name, v, s.is_mutable)?;
            }
            Ok(())
        })())
    }
}

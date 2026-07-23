// eval/attrs.rs — 属性アクセス・代入: メンバーアクセス可能性の検査、get_attr_val / set_attr_val / attr_assign。

use {
    std::cell::RefCell, std::rc::Rc,
    crate::ast::{Accessibility, Expr},
    crate::interpreter::{
        Interpreter, Value,
    },
};

impl Interpreter {
    /// メンバー（フィールド or メソッド）のアクセス可能性を検査する。
    ///
    /// `class` の `field_access` / `method_access` マップで `member_key` を検索し、
    /// アクセス制御に違反する場合は `Err(AccessError: ...)` を返す。
    ///
    /// - `Public`    : 常に OK。
    /// - `Private`   : `self.current_class.name == class.name` のときのみ OK。
    /// - `Protected` : `self.current_class` が同じクラス、またはそのクラスを基底に持つとき OK。
    pub(crate) fn check_member_access(
        &self,
        class: &crate::interpreter::ClassValue,
        member_key: &str,
        display_name: &str,
    ) -> Result<(), String> {
        let access = class
            .field_access
            .get(member_key)
            .or_else(|| class.method_access.get(member_key))
            .cloned()
            .unwrap_or(Accessibility::Public);
        self.check_access_level(class, Self::access_level(&access), display_name)
    }

    /// `Accessibility` を IC 用のレベル定数（0=Public / 1=Private / 2=Protected）へ変換する。
    #[inline]
    pub(crate) fn access_level(access: &Accessibility) -> u8 {
        match access {
            Accessibility::Public => crate::ast::AttrCache::PUBLIC,
            Accessibility::Private => 1,
            Accessibility::Protected => 2,
        }
    }

    /// アクセスレベル（`access_level` で得た u8）だけを使ってアクセス可否を判定する。
    /// R3 インラインキャッシュのヒット経路が `field_access` の辞書引きを飛ばして直接呼ぶ。
    pub(crate) fn check_access_level(
        &self,
        class: &crate::interpreter::ClassValue,
        access: u8,
        display_name: &str,
    ) -> Result<(), String> {
        match access {
            crate::ast::AttrCache::PUBLIC => Ok(()),
            1 => {
                // Private
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name {
                        return Ok(());
                    }
                }
                Err(format!(
                    "AccessError: '{}' is private and cannot be accessed outside '{}'",
                    display_name, class.name
                ))
            }
            _ => {
                // Protected (2)
                if let Some(cur) = &self.current_class {
                    if cur.name == class.name {
                        return Ok(());
                    }
                    // subclass: current_class has class.name in its bases
                    if cur.bases.contains(&class.name) {
                        return Ok(());
                    }
                }
                Err(format!(
                    "AccessError: '{}' is protected and cannot be accessed outside '{}' or its subclasses",
                    display_name, class.name
                ))
            }
        }
    }

    /// Resolve an attribute on any `Value`.
    /// Used by both `eval_attr` (from AST) and native callbacks (`ar_get_attr`).
    ///
    /// `cache` が `Some` かつインスタンスの own/unqualified フィールドに解決できた場合、
    /// `(class_id, slot, アクセスレベル)` を焼き込む（R3・以後は `eval_attr` の高速経路が使う）。
    pub(crate) fn get_attr_val(
        &mut self,
        obj: Value,
        attr: &str,
        cache: Option<&crate::ast::AttrCache>,
    ) -> Result<Value, String> {
        match &obj {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let cls = inst.class.clone();
                if let Some(&idx) = cls.field_index.get(attr) {
                    if let Some(v) = inst.field_value(idx) {
                        // アクセスキーの決定: trait 由来フィールドは修飾名で検索する
                        let suffix = format!("::{attr}");
                        let access_key = cls.field_index.iter()
                            .find(|(k, &i)| k.ends_with(suffix.as_str()) && i == idx)
                            .map(|(k, _)| k.as_str())
                            .unwrap_or(attr);
                        // アクセスレベルを確定して IC に焼く（R3）。
                        let level = {
                            let acc = cls.field_access.get(access_key)
                                .or_else(|| cls.method_access.get(access_key))
                                .cloned()
                                .unwrap_or(Accessibility::Public);
                            Self::access_level(&acc)
                        };
                        if let Some(c) = cache {
                            c.fill(cls.class_id, idx, level);
                        }
                        drop(inst);
                        self.check_access_level(&cls, level, attr)?;
                        return Ok(v);
                    }
                }
                let suffix = format!("::{attr}");
                {
                    let mut trait_matches = cls.field_index.iter()
                        .filter(|(k, _)| k.ends_with(suffix.as_str()));
                    if let Some((full_key, &idx)) = trait_matches.next() {
                        if trait_matches.next().is_some() {
                            // パーサーが静的に検出するはずだが、念のためランタイムでも検出する
                            return Err(format!(
                                "AttributeError: unqualified access to field '{attr}' on '{}' is \
                                 ambiguous (inherited from multiple traits); \
                                 use explicit trait access e.g. `obj:TraitName::{attr}`",
                                cls.name
                            ));
                        }
                        if let Some(v) = inst.field_value(idx) {
                            let full_key = full_key.clone();
                            drop(inst);
                            self.check_member_access(&cls, &full_key, attr)?;
                            return Ok(v);
                        }
                    }
                }
                if let Some(v) = Self::lookup_class_var(&cls, attr) {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr).cloned() {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(cell.borrow().clone());
                }
                if cls.methods.contains_key(attr) {
                    if cls.static_method_names.contains(attr) {
                        drop(inst);
                        return Err(format!(
                            "AttributeError: static method '{}' is not accessible on an instance of '{}'; use '{}.{}'",
                            attr, cls.name, cls.name, attr
                        ));
                    }
                    if cls.class_method_names.contains(attr) {
                        drop(inst);
                        return Err(format!(
                            "AttributeError: class method '{}' is not accessible on an instance of '{}'; use '{}.{}'",
                            attr, cls.name, cls.name, attr
                        ));
                    }
                    let overloads = cls.methods.get(attr).unwrap();
                    let result = if overloads.len() == 1 {
                        Value::Function(overloads[0].clone())
                    } else {
                        Value::OverloadedFn(overloads.clone())
                    };
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(result);
                }
                Err(format!(
                    "AttributeError: '{}' object has no attribute '{attr}'",
                    cls.name
                ))
            }
            Value::Class(cls) => {
                if attr == "name" {
                    return Ok(Value::Str(cls.name.clone()));
                }
                if let Some(v) = Self::lookup_class_var(cls, attr) {
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr) {
                    return Ok(cell.borrow().clone());
                }
                if let Some(overloads) = cls.methods.get(attr) {
                    return Ok(if overloads.len() == 1 {
                        Value::Function(overloads[0].clone())
                    } else {
                        Value::OverloadedFn(overloads.clone())
                    });
                }
                Err(format!(
                    "AttributeError: class '{}' has no attribute '{attr}'",
                    cls.name
                ))
            }
            Value::Namespace(ns) => ns.members.get(attr).cloned().ok_or_else(|| {
                format!(
                    "AttributeError: module '{}' has no attribute '{attr}'",
                    ns.name
                )
            }),
            Value::PyObject(handle) => crate::interpreter::py_interop::py_getattr(handle, attr),
            Value::Slice(s) => match attr {
                "begin" => Ok(s.begin.clone().unwrap_or(Value::None)),
                "end" => Ok(s.end.clone().unwrap_or(Value::None)),
                "step" => Ok(s.step.clone().unwrap_or(Value::None)),
                _ => Err(format!("AttributeError: 'slice' has no attribute '{attr}'")),
            },
            Value::Signal(sig_rc) => {
                // Signal[T] には read-only プロパティのみ。メソッドは eval_method_call が処理する。
                match attr {
                    "handler_count" => Ok(Value::Int(sig_rc.borrow().handlers.len() as i64)),
                    "external_id" => {
                        // 初回アクセス時に発番して external_handler_registry に登録する。
                        // 以後は同じ ID を返す（外部スレッドは ar_event_fire(id, ...) で発火できる）。
                        let existing = sig_rc.borrow().external_id;
                        if let Some(id) = existing {
                            return Ok(Value::Int(id as i64));
                        }
                        let id = self.next_external_signal_id;
                        self.next_external_signal_id += 1;
                        sig_rc.borrow_mut().external_id = Some(id);
                        self.external_handler_registry.insert(id, sig_rc.clone());
                        Ok(Value::Int(id as i64))
                    }
                    _ => Err(format!(
                        "AttributeError: 'Signal' object has no attribute '{attr}'"
                    )),
                }
            }
            Value::EventLoop(_) => {
                // EventLoop のメソッドは eval_method_call が処理する。属性アクセスのみここ。
                Err(format!(
                    "AttributeError: 'EventLoop' object has no attribute '{attr}' (use EventLoop.run() / EventLoop.post())"
                ))
            }
            Value::AsyncManager(mgr_rc) => {
                let mgr = mgr_rc.borrow();
                match attr {
                    "num_thread" => Ok(Value::UInt(mgr.num_thread as u64)),
                    "raise_immediately" => Ok(Value::Bool(mgr.raise_immediately)),
                    "thread_status" => {
                        let running: Vec<Value> = mgr
                            .progress
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| **s == crate::interpreter::async_mgr::AsyncStatus::Running)
                            .map(|(i, _)| Value::Int(i as i64))
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(running))))
                    }
                    "progress_status" => {
                        let statuses: Vec<Value> = mgr
                            .progress
                            .iter()
                            .map(|s| Value::AsyncStatusVal(s.clone()))
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(statuses))))
                    }
                    "results" => Ok(Value::List(Rc::new(RefCell::new(mgr.results.clone())))),
                    "error_list" => {
                        let errs: Vec<Value> = mgr
                            .error_list
                            .iter()
                            .map(|e| match e {
                                Some(s) => Value::Str(s.clone()),
                                None => Value::None,
                            })
                            .collect();
                        Ok(Value::List(Rc::new(RefCell::new(errs))))
                    }
                    _ => Err(format!(
                        "AttributeError: 'AsyncManager' has no attribute '{attr}'"
                    )),
                }
            }
            Value::CsObject(obj_data) => {
                // C# property getter: dispatch as zero-arg instance method.
                let class_name = obj_data.class_name.clone();
                let handle = obj_data.handle;
                let bp = obj_data.bridge_path.clone();
                let is_proc = obj_data.is_proc;
                let class = obj_data.class.clone();
                let ret_type: Option<String> = class
                    .methods
                    .get(attr)
                    .and_then(|ov| ov.first())
                    .and_then(|f| f.return_type.clone());
                if is_proc {
                    crate::interpreter::cs_proc_runtime::call_instance(
                        &bp, &class_name, handle, attr, &[],
                        ret_type.as_deref(),
                    ).map_err(|e| format!("CsProc: property '{attr}' on '{class_name}': {e}"))
                } else {
                    match crate::interpreter::cs_dll_runtime::get_bridge(&bp) {
                        Some(bridge) => crate::interpreter::cs_dll_runtime::call_instance(
                            &bridge, &class_name, handle, attr, &[],
                            ret_type.as_deref(),
                        ).map_err(|e| format!("CsDll: property '{attr}' on '{class_name}': {e}")),
                        None => Err(format!("CsDll: bridge DLL not loaded for '{class_name}'")),
                    }
                }
            }
            _ => Err(format!(
                "AttributeError: '{}' object has no attribute '{attr}'",
                self.type_name(&obj)
            )),
        }
    }

    /// インスタンスの属性に値をセットする。
    /// ネイティブコールバック `ar_set_attr` から呼ばれる。
    pub(crate) fn set_attr_val(
        &mut self,
        obj: Value,
        attr: &str,
        val: Value,
    ) -> Result<(), String> {
        match obj {
            Value::Instance(inst_rc) => {
                let inst_class = inst_rc.borrow().class.clone();
                if Self::lookup_class_var(&inst_class, attr).is_some() {
                    return Err(format!(
                        "TypeError: cannot assign to class variable '{attr}' (declared const)"
                    ));
                }
                self.check_member_access(&inst_class, attr, attr)?;
                let Some(&idx) = inst_class.field_index.get(attr) else {
                    return Err(format!(
                        "AttributeError: '{}' has no field '{attr}'; \
                         all fields must be declared in the class body",
                        inst_class.name
                    ));
                };
                let mut inst = inst_rc.borrow_mut();
                if inst.field_mutable(idx) == Some(false) {
                    return Err(format!(
                        "TypeError: cannot assign to immutable field '{attr}'"
                    ));
                }
                if inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0 {
                    return Err(format!(
                        "TypeError: cannot assign field '{attr}' on immutable instance"
                    ));
                }
                let is_mutable = inst_class.field_mutability.get(attr).copied().unwrap_or(true);
                if !inst.store_field(idx, val, is_mutable) {
                    return Err(format!(
                        "TypeError: value does not match declared type of field '{attr}'"
                    ));
                }
                Ok(())
            }
            _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
        }
    }

    // --- 属性代入ヘルパー ---

    /// 属性・添字に値を代入する。`AttrAssign` 文と `AttrCompoundAssign` 文から呼ばれる。
    ///
    // ---------------------------------------------------------------------------
    // ブロック式 / 制御フロー式 の共通ヘルパー
    // ---------------------------------------------------------------------------

    /// 対応する代入ターゲット:
    /// - `Expr::Attr { object, attr }`: インスタンスフィールドへの代入（可変性・const チェック付き）
    /// - `Expr::TraitAccess { object, trait_name, attr }`: トレイトフィールドへの代入
    /// - `Expr::Subscript { object, index }`: 辞書への添字代入（型制約チェック付き）
    ///
    /// - `target`: 代入先の式（`Attr` / `TraitAccess` / `Subscript`）
    /// - `rhs`: 代入する値（評価済み）
    ///
    /// 戻り値: `Ok(())` — 成功。`Err(message)` — 型エラー・不変フィールドへの代入エラー等
    pub(crate) fn attr_assign(&mut self, target: &Expr, rhs: Value) -> Result<(), String> {
        if let Expr::Attr { object, attr, .. } = target {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    let inst_class = inst_rc.borrow().class.clone();
                    if Self::lookup_class_var(&inst_class, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
                    // static mut 変数への代入: 共有セルを更新する
                    if let Some(cell) = inst_class.static_vars.get(attr.as_str()).cloned() {
                        self.check_member_access(&inst_class, attr, attr)?;
                        *cell.borrow_mut() = rhs;
                        return Ok(());
                    }
                    // アクセス制御チェック
                    self.check_member_access(&inst_class, attr, attr)?;
                    let Some(&idx) = inst_class.field_index.get(attr.as_str()) else {
                        return Err(format!(
                            "AttributeError: '{}' has no field '{attr}'; \
                             all fields must be declared in the class body",
                            inst_class.name
                        ));
                    };
                    let mut inst = inst_rc.borrow_mut();
                    if inst.field_mutable(idx) == Some(false) {
                        return Err(format!(
                            "TypeError: cannot assign to immutable field '{attr}'"
                        ));
                    }
                    if !inst.slot_initialized(idx)
                        && inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0
                    {
                        return Err(format!(
                            "TypeError: cannot assign field '{attr}' on immutable instance"
                        ));
                    }
                    let is_mutable = inst.class.field_mutability.get(attr.as_str()).copied().unwrap_or(true);
                    if !inst.store_field(idx, rhs, is_mutable) {
                        return Err(format!(
                            "TypeError: value does not match declared type of field '{attr}'"
                        ));
                    }
                    Ok(())
                }
                Value::Class(cls) => {
                    // クラスオブジェクトへの代入: static mut 変数のみ許可
                    if let Some(cell) = cls.static_vars.get(attr.as_str()).cloned() {
                        *cell.borrow_mut() = rhs;
                        return Ok(());
                    }
                    if Self::lookup_class_var(&cls, attr).is_some() {
                        return Err(format!(
                            "TypeError: cannot assign to class variable '{attr}' (declared const)"
                        ));
                    }
                    Err(format!(
                        "AttributeError: class '{}' has no static field '{attr}'",
                        cls.name
                    ))
                }
                _ => Err("AttributeError: cannot set attribute on non-instance".to_string()),
            }
        } else if let Expr::TraitAccess {
            object,
            trait_name,
            attr,
        } = target
        {
            let obj_val = self.eval(object)?;
            match obj_val {
                Value::Instance(inst_rc) => {
                    // Trait fields are stored with a namespaced key "TraitName::field"
                    let key = format!("{}::{}", trait_name, attr);
                    let inst_class = inst_rc.borrow().class.clone();
                    // アクセス制御チェック（トレイトフィールドのキーで検索）
                    self.check_member_access(&inst_class, &key, attr)?;
                    let Some(&idx) = inst_class.field_index.get(&key) else {
                        return Err(format!(
                            "AttributeError: trait field '{trait_name}::{attr}' not found on '{}'",
                            inst_class.name
                        ));
                    };
                    let mut inst = inst_rc.borrow_mut();
                    if inst.field_mutable(idx) == Some(false) {
                        return Err(format!(
                            "TypeError: cannot assign to immutable trait field '{attr}'"
                        ));
                    }
                    if inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0 {
                        return Err(format!(
                            "TypeError: cannot assign field '{attr}' on immutable instance"
                        ));
                    }
                    let is_mutable = inst_class.field_mutability.get(&key).copied().unwrap_or(true);
                    if !inst.store_field(idx, rhs, is_mutable) {
                        return Err(format!(
                            "TypeError: value does not match declared type of field '{attr}'"
                        ));
                    }
                    Ok(())
                }
                _ => Err("AttributeError: cannot set trait field on non-instance".to_string()),
            }
        } else if let Expr::Subscript { object, index } = target {
            let obj_val = self.eval(object)?;
            let key = self.eval(index)?;
            self.eval_setitem(obj_val, key, rhs)
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }

}

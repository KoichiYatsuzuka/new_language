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

    /// 名前空間のメンバーを読む（フェーズ10 10-11）。モジュールの `mut` 変数は、写しではなく
    /// **モジュールの大域の今の値**を返す（`NamespaceData::live` の doc）。
    pub(crate) fn namespace_member(
        &self,
        ns: &crate::interpreter::value::NamespaceData,
        name: &str,
    ) -> Option<Value> {
        if let Some((globals, names)) = &ns.live {
            if names.contains(name) {
                // ⚠ 今の大域は `scopes[0]` にある（置き場のその添字は空の置き物・`switch_globals`）。
                let scope = if *globals == self.cur_globals {
                    self.scopes.first()
                } else {
                    self.global_scopes.get(*globals as usize)
                };
                if let Some(v) = scope.and_then(|s| s.get(name)) {
                    return Some(v.get_value());
                }
            }
        }
        ns.members
            .get(name)
            .cloned()
            // サブモジュール（`import a.b` の後の `a.b`・CPython と同じ・`attach_submodule`）。
            .or_else(|| self.submodules.get(&ns.name).and_then(|m| m.get(name)).cloned())
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

    // ── ディスクリプタのプロトコル（python_builtins_plan.md のタスク 1-12）────────────────────────
    //
    // CPython の `object.__getattribute__` / `object.__setattr__` と同じ順:
    //   読み: ① クラス側の**データディスクリプタ**（`__get__` と `__set__` / `__delete__` を持つ）→ `__get__`
    //         ② インスタンスのフィールド ③ クラス側の非データディスクリプタ（`__get__` だけ）→ `__get__`
    //         ④ クラス側の値そのもの ⑤ メソッド（束縛メソッド）
    //   代入: クラス側のデータディスクリプタ → `__set__`。それ以外は従来どおり（フィールド・`static mut`）
    //   クラスから読む（`C.x`）: ディスクリプタなら `__get__(None, C)`（property は自分自身を返す）
    // 「クラス側の値」は `static mut`（Python のクラス属性）とクラス定数。`static mut` は Python のサブクラスへ
    // 共有セルごと引き継がれるので、基底クラスの property もサブクラスのインスタンスから当たる。
    // ⚠ メソッド呼び出し（`obj.x(..)`）はメソッドの無い名前を属性として読んで呼ぶ（タスク 1-11）ので、ここを通る。
    // ⚠ VM の `GetAttr` のキャッシュはフィールドにしか張られないので、ディスクリプタの読みは毎回ここへ来る。

    /// クラス側の値（`static mut` / クラス定数）。
    fn class_level_value(cls: &crate::interpreter::ClassValue, attr: &str) -> Option<Value> {
        if let Some(cell) = cls.static_vars.get(attr) {
            return Some(cell.borrow().clone());
        }
        cls.class_vars.get(attr).cloned()
    }

    /// 値がディスクリプタなら `Some(データディスクリプタか)`。`__get__` を持つインスタンスがディスクリプタ、
    /// さらに `__set__` か `__delete__` を持てばデータディスクリプタ（CPython の `PyDescr_IsData`）。
    pub(crate) fn descriptor_kind(v: &Value) -> Option<bool> {
        let Value::Instance(i) = v else { return None };
        let inst = i.borrow();
        let methods = &inst.class.methods;
        methods
            .contains_key("__get__")
            .then(|| methods.contains_key("__set__") || methods.contains_key("__delete__"))
    }

    /// `desc.__get__(instance, owner)`（クラスから読んだときの `instance` は `None`）。
    /// ⚠ 引数は写さずに渡す（`let` の仮引数でも受け手の書き換えが元へ届くように。`obj.m()` の `self` と同じ）。
    fn descriptor_get(&mut self, desc: Value, instance: Value, owner: Value) -> Result<Value, String> {
        self.eval_method_call_full(desc, "__get__", vec![(None, instance, false), (None, owner, false)], None)
    }

    /// `desc.__set__(instance, value)`。データディスクリプタが `__set__` を持たない（`__delete__` だけ）なら
    /// CPython と同じ `AttributeError: __set__`。
    fn descriptor_set(&mut self, desc: Value, instance: Value, value: Value) -> Result<(), String> {
        let has_set =
            matches!(&desc, Value::Instance(d) if d.borrow().class.methods.contains_key("__set__"));
        if !has_set {
            return Err("AttributeError: __set__".to_string());
        }
        self.eval_method_call_full(desc, "__set__", vec![(None, instance, false), (None, value, false)], None)
            .map(|_| ())
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
        // タスクへ送った、スレッドに縛られる値の複製（7-19。`mng.results` が空のリストを黙って返していた）
        crate::interpreter::async_mgr::ensure_attached(&obj)?;
        match &obj {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                let cls = inst.class.clone();
                // ① クラス側のデータディスクリプタ（property など）はフィールドより先（タスク 1-12）。
                if let Some(desc) = Self::class_level_value(&cls, attr) {
                    if Self::descriptor_kind(&desc) == Some(true) {
                        drop(inst);
                        self.check_member_access(&cls, attr, attr)?;
                        return self.descriptor_get(desc, obj.clone(), Value::Class(cls));
                    }
                }
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
                    // ③ 非データディスクリプタ（`__get__` だけ）はフィールドの後（タスク 1-12）。
                    if Self::descriptor_kind(&v).is_some() {
                        return self.descriptor_get(v, obj.clone(), Value::Class(cls));
                    }
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr).cloned() {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    let v = cell.borrow().clone();
                    if Self::descriptor_kind(&v).is_some() {
                        return self.descriptor_get(v, obj.clone(), Value::Class(cls));
                    }
                    return Ok(v);
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
                    // 束縛メソッド（タスク 1-8）。以前は束縛しない関数を返し、呼ぶと
                    // `missing argument 'self'` / `function takes 1 argument(s), got 0` だった。
                    // ⚠ アクセス制御は**ここで**見る（呼ぶ時点では見ない・`call_bound_method` の doc）。
                    let overloads = cls.methods.get(attr).unwrap().clone();
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(Value::Function(crate::interpreter::FnValue::bind(
                        obj.clone(),
                        crate::interpreter::value::BoundTarget::Methods(overloads),
                    )));
                }
                // `gen` メソッド（タスク 1-10）。呼ぶとジェネレータ（`obj.gen_m(..)` と同じ）。
                if let Some(gen_fn) = cls.gen_methods.get(attr).cloned() {
                    drop(inst);
                    self.check_member_access(&cls, attr, attr)?;
                    return Ok(Value::Function(crate::interpreter::FnValue::bind(
                        obj.clone(),
                        crate::interpreter::value::BoundTarget::Gen(gen_fn),
                    )));
                }
                Err(format!(
                    "AttributeError: '{}' object has no attribute '{attr}'",
                    cls.name
                ))
            }
            Value::Class(cls) => {
                // `__name__` / `__qualname__` は Python の名前（`type(x).__name__`・python_builtins_plan.md のタスク 2-2）。
                // ⚠ 表示名（素の名前）を返す。モジュールの修飾名（`zoo.Dog`）は `__module__` 側の情報で、ここでは付けない。
                if matches!(attr, "name" | "__name__" | "__qualname__") {
                    return Ok(Value::str(cls.name.as_str()));
                }
                // クラスから読むディスクリプタは `__get__(None, C)`（property は自分自身を返す・タスク 1-12）。
                if let Some(v) = Self::lookup_class_var(cls, attr) {
                    if Self::descriptor_kind(&v).is_some() {
                        return self.descriptor_get(v, Value::None, obj.clone());
                    }
                    return Ok(v);
                }
                if let Some(cell) = cls.static_vars.get(attr) {
                    let v = cell.borrow().clone();
                    if Self::descriptor_kind(&v).is_some() {
                        return self.descriptor_get(v, Value::None, obj.clone());
                    }
                    return Ok(v);
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
            // メタ情報への射影（設計書 §1.5 / タスク 4-0）。
            // ⚠⚠ **射影の一覧はここ 1 箇所**（D12）。増やすときは `meta_projection` を直す。
            Value::Meta(m) => match crate::interpreter::value::Projection::from_name(attr) {
                // ⚠ 種別違いの文面に**どの射影か**を書き込む（タスク 3-9）。
                Some(p) => crate::interpreter::value::project(m, p).map_err(|e| {
                    crate::interpreter::value::name_the_projection(e, &format!(".{attr}"))
                }),
                // ⚠ 未知の射影は**黙って `None` を返さない**。綴り間違いが通ってしまう。
                None => Err(format!(
                    "AttributeError: {} '{}' has no projection '{attr}'",
                    self.type_name(&obj),
                    m.name
                )),
            },
            Value::Namespace(ns) => self.namespace_member(ns, attr).ok_or_else(|| {
                format!(
                    "AttributeError: module '{}' has no attribute '{attr}'",
                    ns.name
                )
            }),
            Value::PyObject(handle) => crate::interpreter::py_interop::py_getattr(handle, attr),
            // 型の値の名前（`int` / `list[int]`・D23・タスク 4-2）。型の値から識別子を組み立てるのに使う
            // （`^` の `.type` は型の値を返すので、文字列としては `.name` で取り出す）。
            Value::Type(t) => match attr {
                // `__name__` / `__qualname__` は Python の名前（`type(3).__name__` は `int`・タスク 2-2）。
                "name" | "__name__" | "__qualname__" => Ok(Value::str(t.as_str())),
                _ => Err(format!("AttributeError: type '{t}' has no attribute '{attr}'")),
            },
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
                        let id = self.events.next_external_id;
                        self.events.next_external_id += 1;
                        sig_rc.borrow_mut().external_id = Some(id);
                        self.events.external_handlers.insert(id, sig_rc.clone());
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
                                Some(s) => Value::str(s.as_str()),
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
                let (param_types, ret_type) = crate::interpreter::cs_values::method_types(&class, attr);
                if is_proc {
                    crate::interpreter::cs_proc_runtime::call_instance(
                        &bp, &class_name, handle, attr, &[],
                        ret_type.as_deref(),
                    ).map_err(|e| crate::interpreter::cs_values::tag_error(
                        &format!("CsProc: property '{attr}' on '{class_name}'"), e))
                } else {
                    match crate::interpreter::cs_dll_runtime::get_bridge(&bp) {
                        Some(bridge) => crate::interpreter::cs_dll_runtime::call_instance(
                            &bridge, &class_name, handle, attr, &[],
                            &param_types, ret_type.as_deref(),
                        ).map_err(|e| crate::interpreter::cs_values::tag_error(
                            &format!("CsDll: property '{attr}' on '{class_name}'"), e)),
                        None => Err(format!("CsDll: bridge DLL not loaded for '{class_name}'")),
                    }
                }
            }
            _ => {
                // 組み込みの値のメソッド（`xs.append` / `s.upper` / `d.items`・タスク 1-10）は受け手を束縛した関数。
                // 読める名前は型検査と同じ表（`names::BUILTIN_VALUE_METHODS`）。呼ぶと `eval_method_call_full`。
                let kind = self.type_name(&obj);
                if crate::type_check::names::is_builtin_value_method(kind, attr) {
                    return Ok(Value::Function(crate::interpreter::FnValue::bind(
                        obj.clone(),
                        crate::interpreter::value::BoundTarget::Builtin(attr.to_string()),
                    )));
                }
                Err(format!("AttributeError: '{kind}' object has no attribute '{attr}'"))
            }
        }
    }

    /// インスタンスの属性に値をセットする。
    /// ネイティブコールバック `ar_set_attr` から呼ばれる。
    /// ⚠⚠ **格納する値は関数の入口で複製する**（L4）。「共有するのは `let → let` の
    /// ときだけ」という規則をフィールドへの格納にも通す。共有したままだと
    /// `mut o = N([]); o.f = src` のあと `src.append(9)` で `o.f` が変わり、
    /// `p.f = [p]` で**循環まで作れてしまう**（どちらも実測）。
    ///
    /// ⚠⚠ **複製は `borrow_mut()` を取る前に済ませること。** 値が対象インスタンス自身を
    /// 含む（`p.f = [p]`）と、複製がそのインスタンスを `borrow()` して
    /// `RefCell already mutably borrowed` でパニックする。実際に踏んだ。
    pub(crate) fn set_attr_val(
        &mut self,
        obj: Value,
        attr: &str,
        val: Value,
    ) -> Result<(), String> {
        // ⚠ 借用を取る前に複製する（doc の警告を参照）。
        let val = Self::deep_copy_value(val);
        match obj {
            Value::Instance(inst_rc) => {
                let inst_class = inst_rc.borrow().class.clone();
                // クラス側のデータディスクリプタは `__set__`（`attr_assign_evaled` と同じ・タスク 1-12）。
                if let Some(desc) = Self::class_level_value(&inst_class, attr) {
                    if Self::descriptor_kind(&desc) == Some(true) {
                        self.check_member_access(&inst_class, attr, attr)?;
                        return self.descriptor_set(desc, Value::Instance(inst_rc), val);
                    }
                }
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

    /// 評価済みのオブジェクトと値で属性に代入する。
    ///
    /// `obj.attr = v` の**唯一の実装**（#27-c）。ツリーウォークの `attr_assign` も
    /// `object` を評価してここへ委譲する。以前は `attr_assign` 側に `Value::Class`
    /// （`static mut` への代入）のアームが**もう 1 つ**あり、VM 側にだけ無かった。
    /// コンパイラが「レシーバが Arrow インスタンスと確定できるときだけ `SetAttr`」と
    /// 絞ることで差を隠していたが、絞れないレシーバ（型注釈の無いグローバル等）が
    /// そのまま bail になっていた。
    /// ⚠ 格納する値は**入口で**複製する（L4。`set_attr_val` の doc と同じ理由・同じ罠）。
    pub(crate) fn attr_assign_evaled(
        &mut self,
        obj: Value,
        attr: &str,
        rhs: Value,
    ) -> Result<(), String> {
        // ⚠ 借用を取る前に複製する（doc の警告を参照）。
        let rhs = Self::deep_copy_value(rhs);
        match obj {
            Value::Instance(inst_rc) => {
                let inst_class = inst_rc.borrow().class.clone();
                // クラス側のデータディスクリプタへの代入は `__set__(obj, v)`（property の setter・タスク 1-12）。
                // `static mut` の書き換え・フィールドより先（CPython の `object.__setattr__` と同じ）。
                if let Some(desc) = Self::class_level_value(&inst_class, attr) {
                    if Self::descriptor_kind(&desc) == Some(true) {
                        self.check_member_access(&inst_class, attr, attr)?;
                        return self.descriptor_set(desc, Value::Instance(inst_rc), rhs);
                    }
                }
                if Self::lookup_class_var(&inst_class, attr).is_some() {
                    return Err(format!(
                        "TypeError: cannot assign to class variable '{attr}' (declared const)"
                    ));
                }
                // `import[rs]` の構造体のフィールド: Rust の値そのものを書き換える（生成した `set_{field}` が
                // 構造体と、Arrow 側に写したフィールドの両方を更新する・ffi_value_exchange_plan.md 2-5）。
                // ⚠ 以前は Arrow 側の写しだけが変わり、Rust のメソッドは古い値を見ていた。
                // ⚠ ネイティブ側からの `set_attr`（写しの更新）は `set_attr_val` を通るので、ここへは来ない（再入しない）。
                if inst_class.field_index.contains_key("__rs_handle__") && attr != "__rs_handle__" {
                    let setter = format!("set_{attr}");
                    if crate::interpreter::native_api::lookup_native_method_ptr(&inst_class.name, &setter).is_some() {
                        self.check_member_access(&inst_class, attr, attr)?;
                        return match crate::interpreter::native_api::try_dispatch_native_method(
                            self,
                            Value::Instance(inst_rc.clone()),
                            &setter,
                            vec![rhs],
                        ) {
                            Some(r) => r.map(|_| ()),
                            None => Err(format!("NativeError: setter '{setter}' of '{}' is not loaded", inst_class.name)),
                        };
                    }
                }
                // static mut 変数への代入: 共有セルを更新する
                if let Some(cell) = inst_class.static_vars.get(attr).cloned() {
                    self.check_member_access(&inst_class, attr, attr)?;
                    *cell.borrow_mut() = rhs;
                    return Ok(());
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
                if !inst.slot_initialized(idx)
                    && inst.flags() & crate::interpreter::value::INST_IMMUTABLE != 0
                {
                    return Err(format!(
                        "TypeError: cannot assign field '{attr}' on immutable instance"
                    ));
                }
                let is_mutable = inst.class.field_mutability.get(attr).copied().unwrap_or(true);
                if !inst.store_field(idx, rhs, is_mutable) {
                    return Err(format!(
                        "TypeError: value does not match declared type of field '{attr}'"
                    ));
                }
                Ok(())
            }
            Value::Class(cls) => {
                // クラスオブジェクトへの代入: static mut 変数のみ許可
                if let Some(cell) = cls.static_vars.get(attr).cloned() {
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
            // ⚠⚠ **import したモジュールの `mut` のグローバル変数は外から代入できる**（CPython と同じ・
            //    `implementation_plans/enum_member_type_plan.md` 6-2）。書くのはモジュールの大域そのもの
            //    （`namespace_member` が今の値を読む所）。以前は名前空間への代入に腕が無く、`mut` でも
            //    `cannot set attribute on non-instance` になっていた。
            //    `let` / `const`・関数などは型検査が弾く（6-1）。ここは型検査を通らない経路の砦。
            Value::Namespace(ns) => {
                if let Some((globals, names)) = &ns.live {
                    if names.contains(attr) {
                        // ⚠ 今の大域は `scopes[0]` にある（`namespace_member` と同じ）。
                        let scope = if *globals == self.cur_globals {
                            self.scopes.first_mut()
                        } else {
                            self.global_scopes.get_mut(*globals as usize)
                        };
                        if let Some(var) = scope.and_then(|s| s.get_mut(attr)) {
                            if var.is_mutable() {
                                var.set_value(rhs);
                                return Ok(());
                            }
                        }
                    }
                }
                if self.namespace_member(&ns, attr).is_some() {
                    return Err(format!(
                        "TypeError: cannot assign to '{}.{attr}': only `mut` module variables can be assigned",
                        ns.name
                    ));
                }
                Err(format!("AttributeError: module '{}' has no attribute '{attr}'", ns.name))
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
    /// - `Expr::Subscript { object, index, .. }`: 辞書への添字代入（型制約チェック付き）
    ///
    /// - `target`: 代入先の式（`Attr` / `TraitAccess` / `Subscript`）
    /// - `rhs`: 代入する値（評価済み）
    ///
    /// 戻り値: `Ok(())` — 成功。`Err(message)` — 型エラー・不変フィールドへの代入エラー等
    pub(crate) fn attr_assign(&mut self, target: &Expr, rhs: Value) -> Result<(), String> {
        if let Expr::Attr { object, attr, .. } = target {
            let obj_val = self.eval(object)?;
            self.attr_assign_evaled(obj_val, attr, rhs)
        } else if let Expr::TraitAccess {
            object,
            trait_name,
            attr,
        } = target
        {
            let obj_val = self.eval(object)?;
            self.trait_assign_evaled(obj_val, trait_name, attr, rhs)
        } else if let Expr::Subscript { object, index, .. } = target {
            let obj_val = self.eval(object)?;
            let key = self.eval(index)?;
            self.eval_setitem(obj_val, key, rhs)
        } else {
            Err("SyntaxError: invalid assignment target".to_string())
        }
    }

    /// `obj::Trait.attr = value` の代入（評価済みレシーバ版・#27）。
    ///
    /// VM の `Op::SetTraitAttr` とツリーウォークの `attr_assign` の**唯一の実装**。
    /// 可変性検査・不変インスタンス検査・宣言型検査はすべてここに集約する
    /// （`*_evaled` 版とずれた実装を作らない — #22 系列）。
    /// ⚠ `#[inline(never)]`: トレイトフィールド代入は稀なので `attr_assign` へ展開させない。
    #[inline(never)]
    /// ⚠ 格納する値は**入口で**複製する（L4。`set_attr_val` の doc と同じ理由・同じ罠）。
    pub(crate) fn trait_assign_evaled(
        &mut self,
        obj_val: Value,
        trait_name: &str,
        attr: &str,
        rhs: Value,
    ) -> Result<(), String> {
        // ⚠ 借用を取る前に複製する（doc の警告を参照）。
        let rhs = Self::deep_copy_value(rhs);
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
    }

}
